//! Opt-in: `install` with the real npm and the real launcher against a
//! loopback registry that serves fabricated packages. No network beyond
//! `127.0.0.1`, no credential and no model.
//!
//! ```console
//! cargo test -p vsift-agent-trials --test install_npm -- --ignored --nocapture
//! ```
//!
//! It needs Node.js 22 or later with npm: `VSIFT_TEST_NODE` and
//! `VSIFT_TEST_NPM_CLI` (absolute paths), or `node` on `PATH` with npm beside
//! it. It proves what the fabricated-tree tests cannot: that the layout, the
//! hidden lockfile, the shim, `npm view ... dist.integrity --json` and the
//! launcher's digest check are what the real npm and the real launcher
//! produce, and that npm, started by the harness, sends the registry nothing
//! but its own client headers (no credential, no cookie).

mod common;

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use common::{Scratch, repository};
use serde_json::json;
use sha2::{Digest, Sha512};
use vsift_agent_trials::{
    install::{InstallRequest, host_target, install},
    roots::RootPolicy,
};

type TestResult = Result<(), Box<dyn Error>>;

const VERSION: &str = "0.1.0";
const STUB: &str = env!("CARGO_BIN_EXE_vsift-trials-stub-client");

/// Node.js and npm's script, from the environment or beside `node`.
fn node_and_npm() -> Option<(PathBuf, PathBuf)> {
    let node = env::var_os("VSIFT_TEST_NODE")
        .map(PathBuf::from)
        .or_else(|| {
            env::split_paths(&env::var_os("PATH")?)
                .map(|directory| directory.join(if cfg!(windows) { "node.exe" } else { "node" }))
                .find(|candidate| candidate.is_file())
        })?;
    let npm_cli = env::var_os("VSIFT_TEST_NPM_CLI")
        .map(PathBuf::from)
        .or_else(|| {
            let directory = node.parent()?;
            [
                directory.join("node_modules/npm/bin/npm-cli.js"),
                directory.join("../lib/node_modules/npm/bin/npm-cli.js"),
            ]
            .into_iter()
            .find(|candidate| candidate.is_file())
        })?;
    Some((node.canonicalize().ok()?, npm_cli.canonicalize().ok()?))
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let value = chunk.iter().enumerate().fold(0_u32, |acc, (index, byte)| {
            acc | (u32::from(*byte) << (16 - 8 * index))
        });
        for index in 0..4 {
            if index <= chunk.len() {
                let part = usize::try_from((value >> (18 - 6 * index)) & 0x3f).unwrap_or(0);
                text.push(char::from(ALPHABET[part]));
            } else {
                text.push('=');
            }
        }
    }
    text
}

fn integrity(bytes: &[u8]) -> String {
    format!("sha512-{}", base64(&Sha512::digest(bytes)))
}

/// `npm pack` of a folder, offline, into `destination`; the tarball's path.
fn pack(
    node: &Path,
    npm_cli: &Path,
    folder: &Path,
    destination: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    fs::create_dir_all(destination)?;
    let home = destination.join("pack-home");
    fs::create_dir_all(&home)?;
    let mut command = Command::new(node);
    command
        .arg(npm_cli)
        .args([
            "pack",
            "--ignore-scripts",
            "--json",
            "--loglevel=error",
            "--pack-destination",
        ])
        .arg(destination)
        .arg(folder)
        .env_clear()
        .env("PATH", node.parent().unwrap_or(Path::new("")))
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("npm_config_cache", home.join("cache"))
        .env("npm_config_userconfig", home.join("user.npmrc"))
        .env("npm_config_globalconfig", home.join("global.npmrc"))
        .stdin(Stdio::null());
    for name in ["SystemRoot", "windir", "ComSpec", "PATHEXT", "TEMP", "TMP"] {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "npm pack failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let packed: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let name = packed[0]["filename"]
        .as_str()
        .ok_or("npm pack named no file")?;
    Ok(destination.join(name))
}

/// A loopback registry: fixed routes, and every request's headers kept.
struct Registry {
    port: u16,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    handle: Option<JoinHandle<()>>,
}

fn respond(
    mut stream: TcpStream,
    routes: &BTreeMap<String, (String, Vec<u8>)>,
    requests: &Mutex<Vec<String>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 2048];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") && buffer.len() < 32 * 1024 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
        }
    }
    let head = String::from_utf8_lossy(&buffer).into_owned();
    if let Ok(mut seen) = requests.lock() {
        seen.push(head.clone());
    }
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .replace("%2f", "/")
        .replace("%2F", "/");
    let (status, content_type, body) = match routes.get(&path) {
        Some((content_type, body)) => ("200 OK", content_type.as_str(), body.clone()),
        None => (
            "404 Not Found",
            "application/json",
            b"{\"error\":\"not found\"}".to_vec(),
        ),
    };
    let header = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&body);
}

impl Registry {
    fn start(
        routes: impl FnOnce(u16) -> BTreeMap<String, (String, Vec<u8>)>,
    ) -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let routes = routes(port);
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (flag, log) = (Arc::clone(&stop), Arc::clone(&requests));
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        respond(stream, &routes, &log);
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Ok(Self {
            port,
            stop,
            requests,
            handle: Some(handle),
        })
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    fn finish(mut self) -> Vec<String> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.requests
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

#[test]
#[ignore = "opt-in: needs Node.js and npm; serves fabricated packages from 127.0.0.1"]
#[allow(
    clippy::too_many_lines,
    reason = "One test reads top to bottom as the order of the install it checks"
)]
fn the_real_npm_installs_from_a_registry_and_the_proof_agrees() -> TestResult {
    let Some((node, npm_cli)) = node_and_npm() else {
        return Err("no Node.js with npm: set VSIFT_TEST_NODE and VSIFT_TEST_NPM_CLI".into());
    };
    let scratch = Scratch::new("install-npm")?;
    let target = host_target()?;
    let packages = scratch.path().join("packages");

    // The launcher package: the real launcher, a fabricated digest record and
    // a skill folder, as the published package is laid out.
    let launcher = packages.join("launcher");
    for file in ["bin/vsift.cjs", "lib/launcher.cjs"] {
        let destination = launcher.join(file);
        fs::create_dir_all(destination.parent().ok_or("no parent")?)?;
        fs::copy(repository().join("npm/vsift-cli").join(file), destination)?;
    }
    let native_bytes = fs::read(STUB)?;
    fs::write(
        launcher.join("platform-digests.json"),
        json!({
            "format": "vsift-platform-digests/1", "version": VERSION,
            "packages": {target.package: {
                "file": target.executable, "size": native_bytes.len(),
                "sha256": vsift_agent_trials::skill::sha256_hex(&native_bytes)}}
        })
        .to_string(),
    )?;
    fs::create_dir_all(launcher.join("skills/vsift"))?;
    fs::write(launcher.join("skills/vsift/SKILL.md"), "the skill\n")?;
    fs::write(launcher.join("README.md"), "the readme\n")?;
    let (os, cpu) = match target.package {
        "@vsift/win32-x64" => ("win32", "x64"),
        "@vsift/linux-x64" => ("linux", "x64"),
        _ => ("darwin", "arm64"),
    };
    let launcher_manifest = json!({
        "name": "vsift-cli", "version": VERSION, "bin": {"vsift": "bin/vsift.cjs"},
        "files": ["bin/vsift.cjs", "lib/launcher.cjs", "platform-digests.json", "skills/", "README.md"],
        "engines": {"node": ">=22"},
        "optionalDependencies": {
            "@vsift/darwin-arm64": VERSION, "@vsift/linux-x64": VERSION, "@vsift/win32-x64": VERSION}
    });
    fs::write(launcher.join("package.json"), launcher_manifest.to_string())?;

    // This platform's native package: the stand-in as the executable.
    let native = packages.join("native");
    fs::create_dir_all(&native)?;
    fs::write(native.join(target.executable), &native_bytes)?;
    fs::write(
        native.join("package.json"),
        json!({"name": target.package, "version": VERSION, "os": [os], "cpu": [cpu],
               "files": [target.executable]})
        .to_string(),
    )?;
    let tarballs = packages.join("tarballs");
    let launcher_tarball = fs::read(pack(&node, &npm_cli, &launcher, &tarballs)?)?;
    let native_tarball = fs::read(pack(&node, &npm_cli, &native, &tarballs)?)?;
    let (launcher_sri, native_sri) = (integrity(&launcher_tarball), integrity(&native_tarball));

    let platform_path = target.package.to_owned();
    let platform_leaf = target
        .package
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let registry = Registry::start(|port| {
        let base = format!("http://127.0.0.1:{port}");
        let json_type = "application/json".to_owned();
        let packument = |name: &str, manifest: &serde_json::Value, tarball: &str, sri: &str| {
            let mut version = manifest.clone();
            version["dist"] = json!({"tarball": format!("{base}{tarball}"), "integrity": sri});
            json!({"name": name, "dist-tags": {"next": VERSION}, "versions": {VERSION: version}})
                .to_string()
        };
        let mut native_manifest =
            json!({"name": platform_path, "version": VERSION, "os": [os], "cpu": [cpu]});
        native_manifest["dist"] = json!({});
        BTreeMap::from([
            (
                "/vsift-cli".to_owned(),
                (
                    json_type.clone(),
                    packument(
                        "vsift-cli",
                        &launcher_manifest,
                        "/vsift-cli/-/vsift-cli-0.1.0.tgz",
                        &launcher_sri,
                    )
                    .into_bytes(),
                ),
            ),
            (
                format!("/{platform_path}"),
                (
                    json_type,
                    packument(
                        &platform_path,
                        &native_manifest,
                        &format!("/{platform_path}/-/{platform_leaf}-0.1.0.tgz"),
                        &native_sri,
                    )
                    .into_bytes(),
                ),
            ),
            (
                "/vsift-cli/-/vsift-cli-0.1.0.tgz".to_owned(),
                (
                    "application/octet-stream".to_owned(),
                    launcher_tarball.clone(),
                ),
            ),
            (
                format!("/{platform_path}/-/{platform_leaf}-0.1.0.tgz"),
                (
                    "application/octet-stream".to_owned(),
                    native_tarball.clone(),
                ),
            ),
        ])
    })?;

    let proof = install(&InstallRequest {
        version: VERSION.to_owned(),
        prefix: scratch.path().join("prefix"),
        node,
        npm_cli,
        registry: registry.url(),
        pass_environment: Vec::new(),
        root_policy: RootPolicy::new(Vec::new(), Vec::new()),
    });
    let seen = registry.finish();
    let proof = proof?;
    let evidence = &proof.evidence;

    assert_eq!(
        evidence.not_published_because(),
        Vec::<String>::new(),
        "{evidence:?}"
    );
    assert_eq!(evidence.packages.len(), 2);
    assert_eq!(evidence.packages[0].integrity_installed, launcher_sri);
    assert_eq!(evidence.packages[0].integrity_registry, launcher_sri);
    assert_eq!(evidence.packages[1].integrity_installed, native_sri);
    assert!(evidence.launcher.matches);
    assert_eq!(evidence.launcher.version_exit_code, Some(0));
    // What the real npm wrote for the command: three files on Windows (the
    // POSIX script first), the one link elsewhere (#257).
    assert_eq!(
        evidence.command_shims,
        if cfg!(windows) {
            vec!["vsift", "vsift.cmd", "vsift.ps1"]
        } else {
            vec!["vsift"]
        }
    );
    assert_eq!(evidence.launcher.version_line, "vsift 0.1.0 (0123456789ab)");
    assert!(proof.native_executable.is_file());
    assert!(proof.package_root.join("skills/vsift/SKILL.md").is_file());
    assert!(
        !scratch.path().join("prefix.support").exists(),
        "the scratch folder was removed"
    );

    // npm, started by the harness, sent the registry only npm's own headers.
    assert!(!seen.is_empty(), "the registry saw nothing");
    for request in &seen {
        let lowered = request.to_lowercase();
        for forbidden in ["authorization:", "cookie:", "x-forwarded-for:"] {
            assert!(!lowered.contains(forbidden), "{forbidden} in {request}");
        }
        assert!(
            lowered
                .lines()
                .any(|line| line.starts_with("user-agent: npm/")),
            "{request}"
        );
        for name in ["USERNAME", "USER", "LOGNAME"] {
            if let Ok(user) = env::var(name)
                && user.chars().count() >= 3
            {
                assert!(
                    !lowered.contains(&user.to_lowercase()),
                    "the user name reached the registry"
                );
            }
        }
    }
    Ok(())
}
