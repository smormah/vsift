//! Clean-install mode: what an install must show to count as the published
//! package, against a fabricated install tree (no npm, no network).
//!
//! The tree has the shape the real npm gives a global install (checked with
//! npm 11.4.2 by the opt-in `install_npm` test): the launcher package below
//! `node_modules`, this platform's native package nested inside it, the
//! launcher's `platform-digests.json` and the command shim. A fake registry
//! answers what `npm view ... dist` would, a fake index answers what npm's
//! cache holds, and a fake probe answers `vsift --version` through the
//! launcher.

mod common;

use std::{collections::BTreeMap, error::Error, fs, path::PathBuf};

use common::Scratch;
use serde_json::json;
use vsift_agent_trials::{
    TrialError,
    install::{
        CacheIndex, Dist, InstallEvidence, InstallProof, InstallSourceKind, InstalledIntegrity,
        LAUNCHER_PACKAGE, RegistryReader, Target, VerifyRequest, VersionProbe, command_directory,
        host_target, modules_root, target_for, validate_registry, validate_version, verify_install,
    },
    skill::sha256_hex,
};

type TestResult = Result<(), Box<dyn Error>>;

const REGISTRY: &str = "https://registry.npmjs.org/";
const VERSION: &str = "0.1.0";
const LAUNCHER_INTEGRITY: &str = "sha512-launcherlauncherlauncherlauncherlauncherlauncherlauncherlauncherlauncherlauncherlauncherA==";
const NATIVE_INTEGRITY: &str = "sha512-nativenativenativenativenativenativenativenativenativenativenativenativenativenativenatA==";
const OTHER_INTEGRITY: &str =
    "sha512-differentdifferentdifferentdifferentdifferentdifferentdifferentdifferentdifferentdA==";
const NATIVE_BYTES: &[u8] = b"the native executable";

/// What `npm view <package>@<version> dist --json` would answer.
struct FakeRegistry(BTreeMap<String, Dist>);

impl RegistryReader for FakeRegistry {
    fn dist(&self, name: &str, version: &str) -> Result<Dist, TrialError> {
        self.0
            .get(&format!("{name}@{version}"))
            .cloned()
            .ok_or_else(|| TrialError::Process(format!("the registry has no {name}@{version}")))
    }
}

/// What npm's cache index holds, by tarball address.
struct FakeInstalled(BTreeMap<String, String>);

impl InstalledIntegrity for FakeInstalled {
    fn integrity_of(&self, tarball: &str) -> Result<String, TrialError> {
        self.0.get(tarball).cloned().ok_or_else(|| {
            TrialError::Invalid("npm has no record of fetching a tarball".to_owned())
        })
    }
}

/// What `vsift --version` through the launcher would answer.
struct FakeProbe {
    exit: Option<i32>,
    line: String,
}

impl VersionProbe for FakeProbe {
    fn version(&self, _: &std::path::Path) -> Result<(Option<i32>, String), TrialError> {
        Ok((self.exit, self.line.clone()))
    }
}

fn tarball(host: &str, name: &str) -> String {
    format!(
        "{host}{name}/-/{}-{VERSION}.tgz",
        name.rsplit('/').next().unwrap_or(name)
    )
}

/// The registry's answers for the launcher and this platform's package, and
/// what npm's cache holds when it fetched exactly those.
fn consistent(target: Target, host: &str) -> (FakeRegistry, FakeInstalled) {
    let packages = [
        (LAUNCHER_PACKAGE, LAUNCHER_INTEGRITY),
        (target.package, NATIVE_INTEGRITY),
    ];
    let registry = FakeRegistry(
        packages
            .iter()
            .map(|(name, integrity)| {
                (
                    format!("{name}@{VERSION}"),
                    Dist {
                        tarball: tarball(host, name),
                        integrity: (*integrity).to_owned(),
                    },
                )
            })
            .collect(),
    );
    let installed = FakeInstalled(
        packages
            .iter()
            .map(|(name, integrity)| (tarball(host, name), (*integrity).to_owned()))
            .collect(),
    );
    (registry, installed)
}

fn probe() -> FakeProbe {
    FakeProbe {
        exit: Some(0),
        line: format!("vsift {VERSION} (0123456789ab)"),
    }
}

/// A fabricated install below a scratch folder.
struct Fabricated {
    scratch: Scratch,
    prefix: PathBuf,
    target: Target,
    node: PathBuf,
}

impl Fabricated {
    /// The platform package nested in the launcher's folder, as npm does
    /// for a global install, or beside it (`hoisted`).
    fn new(hoisted: bool) -> Result<Self, Box<dyn Error>> {
        let scratch = Scratch::new("install")?;
        let target = host_target()?;
        let prefix = scratch.path().join("prefix");
        let modules = modules_root(&prefix);
        let launcher = modules.join(LAUNCHER_PACKAGE);
        let relative: PathBuf = target.package.split('/').collect();
        let native = if hoisted {
            modules.join(&relative)
        } else {
            launcher.join("node_modules").join(&relative)
        };
        fs::create_dir_all(&launcher)?;
        fs::create_dir_all(&native)?;
        fs::write(
            launcher.join("package.json"),
            json!({"name": LAUNCHER_PACKAGE, "version": VERSION}).to_string(),
        )?;
        fs::write(
            native.join("package.json"),
            json!({"name": target.package, "version": VERSION}).to_string(),
        )?;
        fs::write(native.join(target.executable), NATIVE_BYTES)?;
        fs::write(
            launcher.join("platform-digests.json"),
            json!({
                "format": "vsift-platform-digests/1",
                "version": VERSION,
                "packages": {target.package: {
                    "file": target.executable,
                    "size": NATIVE_BYTES.len(),
                    "sha256": sha256_hex(NATIVE_BYTES),
                }}
            })
            .to_string(),
        )?;
        let commands = command_directory(&prefix);
        fs::create_dir_all(&commands)?;
        fs::write(
            commands.join(if cfg!(windows) { "vsift.cmd" } else { "vsift" }),
            "shim",
        )?;
        let node = scratch.path().join("node").join("node");
        fs::create_dir_all(scratch.path().join("node"))?;
        fs::write(&node, "node")?;
        Ok(Self {
            scratch,
            prefix,
            target,
            node,
        })
    }

    fn proof(
        &self,
        registry: &dyn RegistryReader,
        installed: &dyn InstalledIntegrity,
        probe: &dyn VersionProbe,
    ) -> Result<InstallProof, TrialError> {
        verify_install(
            &VerifyRequest {
                prefix: &self.prefix,
                version: VERSION,
                registry: REGISTRY,
                source: InstallSourceKind::Registry,
                node: &self.node,
                node_version: "v24.21.0",
                npm_version: "11.19.0",
                target: self.target,
            },
            registry,
            installed,
            probe,
        )
    }

    fn native(&self, hoisted: bool) -> PathBuf {
        let relative: PathBuf = self.target.package.split('/').collect();
        let base = if hoisted {
            modules_root(&self.prefix)
        } else {
            modules_root(&self.prefix)
                .join(LAUNCHER_PACKAGE)
                .join("node_modules")
        };
        base.join(relative).join(self.target.executable)
    }
}

#[test]
fn a_registry_install_with_matching_records_counts_as_published() -> TestResult {
    let fabricated = Fabricated::new(false)?;
    let (registry, installed) = consistent(fabricated.target, REGISTRY);
    let proof = fabricated.proof(&registry, &installed, &probe())?;
    assert_eq!(
        proof.evidence.not_published_because(),
        Vec::<String>::new(),
        "{:?}",
        proof.evidence
    );
    let evidence = &proof.evidence;
    assert_eq!(evidence.version, VERSION);
    assert_eq!(evidence.packages.len(), 2);
    assert_eq!(evidence.packages[0].name, LAUNCHER_PACKAGE);
    assert_eq!(
        evidence.packages[0].resolved,
        "https://registry.npmjs.org/vsift-cli/-/vsift-cli-0.1.0.tgz"
    );
    assert_eq!(evidence.packages[1].integrity_registry, NATIVE_INTEGRITY);
    assert_eq!(evidence.packages[1].integrity_installed, NATIVE_INTEGRITY);
    assert!(evidence.launcher.matches);
    assert_eq!(evidence.launcher.actual_sha256, sha256_hex(NATIVE_BYTES));
    assert_eq!(evidence.launcher.version_line, "vsift 0.1.0 (0123456789ab)");
    assert!(evidence.ignore_scripts);
    // The agent reaches vsift through npm's shim and Node.js, never the
    // native executable's own folder.
    assert_eq!(proof.client_path_directories.len(), 2);
    assert_eq!(
        proof.client_path_directories[0],
        command_directory(&fabricated.prefix)
    );
    assert_eq!(proof.native_executable, fabricated.native(false));
    // Evidence is path-free: it may enter a record.
    let text = serde_json::to_string(evidence)?;
    assert!(
        !text.contains(
            &fabricated
                .scratch
                .path()
                .to_string_lossy()
                .replace('\\', "\\\\")
        ),
        "{text}"
    );
    Ok(())
}

#[test]
fn a_hoisted_layout_is_found_as_well_as_the_nested_one() -> TestResult {
    let fabricated = Fabricated::new(true)?;
    let (registry, installed) = consistent(fabricated.target, REGISTRY);
    let proof = fabricated.proof(&registry, &installed, &probe())?;
    assert_eq!(proof.native_executable, fabricated.native(true));
    assert!(proof.evidence.not_published_because().is_empty());
    Ok(())
}

/// Each way an install can fail to be the published package is named.
#[test]
fn every_way_an_install_is_not_the_published_one_is_named() -> TestResult {
    let reasons = |proof: &InstallProof| proof.evidence.not_published_because();

    // npm fetched other bytes than the registry advertises.
    let fabricated = Fabricated::new(false)?;
    let (registry, mut installed) = consistent(fabricated.target, REGISTRY);
    installed.0.insert(
        tarball(REGISTRY, LAUNCHER_PACKAGE),
        OTHER_INTEGRITY.to_owned(),
    );
    let proof = fabricated.proof(&registry, &installed, &probe())?;
    assert!(
        reasons(&proof)
            .iter()
            .any(|reason| reason.contains("integrity is not the registry's")),
        "{:?}",
        reasons(&proof)
    );

    // Tarballs advertised from somewhere other than the registry asked.
    let elsewhere = Fabricated::new(false)?;
    let (registry, installed) = consistent(elsewhere.target, "https://registry.example.com/");
    let proof = elsewhere.proof(&registry, &installed, &probe())?;
    assert!(
        reasons(&proof)
            .iter()
            .any(|reason| reason.contains("not resolved from the registry")),
        "{:?}",
        reasons(&proof)
    );

    // The native executable differs from the launcher's recorded digest.
    let tampered = Fabricated::new(false)?;
    fs::write(tampered.native(false), b"replaced bytes")?;
    let (registry, installed) = consistent(tampered.target, REGISTRY);
    let proof = tampered.proof(&registry, &installed, &probe())?;
    assert!(!proof.evidence.launcher.matches);
    assert!(
        reasons(&proof)
            .iter()
            .any(|reason| reason.contains("does not match the launcher's digest")),
        "{:?}",
        reasons(&proof)
    );

    // The launcher refused (126), or named another version.
    let healthy = Fabricated::new(false)?;
    let (registry, installed) = consistent(healthy.target, REGISTRY);
    let refused = FakeProbe {
        exit: Some(126),
        line: String::new(),
    };
    assert!(
        reasons(&healthy.proof(&registry, &installed, &refused)?)
            .iter()
            .any(|reason| reason.contains("did not exit 0"))
    );
    let wrong = FakeProbe {
        exit: Some(0),
        line: "vsift 0.9.9 (0123456789ab)".to_owned(),
    };
    assert!(
        reasons(&healthy.proof(&registry, &installed, &wrong)?)
            .iter()
            .any(|reason| reason.contains("does not name the installed version"))
    );
    Ok(())
}

#[test]
fn a_package_of_another_version_or_a_missing_record_is_an_error() -> TestResult {
    let fabricated = Fabricated::new(false)?;
    let (registry, installed) = consistent(fabricated.target, REGISTRY);
    let launcher = modules_root(&fabricated.prefix).join(LAUNCHER_PACKAGE);
    fs::write(
        launcher.join("package.json"),
        json!({"name": LAUNCHER_PACKAGE, "version": "0.1.1"}).to_string(),
    )?;
    let error = fabricated
        .proof(&registry, &installed, &probe())
        .err()
        .ok_or("a launcher of another version was accepted")?;
    assert!(
        error.to_string().contains("not the requested 0.1.0"),
        "{error}"
    );

    // npm has no record of fetching a package the registry lists.
    let unrecorded = Fabricated::new(false)?;
    let (registry, _) = consistent(unrecorded.target, REGISTRY);
    let error = unrecorded
        .proof(&registry, &FakeInstalled(BTreeMap::new()), &probe())
        .err()
        .ok_or("an install npm never fetched was accepted")?;
    assert!(
        error.to_string().contains("no record of fetching"),
        "{error}"
    );

    // The registry does not know the version.
    let unknown = Fabricated::new(false)?;
    let (_, installed) = consistent(unknown.target, REGISTRY);
    assert!(
        unknown
            .proof(&FakeRegistry(BTreeMap::new()), &installed, &probe())
            .is_err()
    );

    let no_shim = Fabricated::new(false)?;
    for entry in fs::read_dir(command_directory(&no_shim.prefix))? {
        let path = entry?.path();
        if path.is_file() {
            fs::remove_file(path)?;
        }
    }
    let (registry, installed) = consistent(no_shim.target, REGISTRY);
    let error = no_shim
        .proof(&registry, &installed, &probe())
        .err()
        .ok_or("an install without a command shim was accepted")?;
    assert!(error.to_string().contains("no vsift command"), "{error}");
    Ok(())
}

#[test]
fn a_proof_is_reloaded_only_while_the_executable_is_unchanged() -> TestResult {
    let fabricated = Fabricated::new(false)?;
    let (registry, installed) = consistent(fabricated.target, REGISTRY);
    let proof = fabricated.proof(&registry, &installed, &probe())?;
    let file = fabricated.scratch.path().join("proof.json");
    proof.write(&file)?;
    assert_eq!(InstallProof::load(&file)?, proof);

    fs::write(fabricated.native(false), b"replaced after the proof")?;
    let error = InstallProof::load(&file)
        .err()
        .ok_or("a changed executable was accepted")?;
    assert!(
        error
            .to_string()
            .contains("not the one the install proof recorded"),
        "{error}"
    );

    // A proof that does not count as published is refused when loaded.
    let other = Fabricated::new(false)?;
    let (registry, mut installed) = consistent(other.target, REGISTRY);
    installed.0.insert(
        tarball(REGISTRY, other.target.package),
        OTHER_INTEGRITY.to_owned(),
    );
    let proof = other.proof(&registry, &installed, &probe())?;
    let file = other.scratch.path().join("proof.json");
    proof.write(&file)?;
    let error = InstallProof::load(&file)
        .err()
        .ok_or("an unverified install was accepted")?;
    assert!(
        error.to_string().contains("not the published package"),
        "{error}"
    );
    Ok(())
}

/// npm's cache index, `cacache` v5, as the real npm writes it (the opt-in
/// `install_npm` test reads a real one).
fn cache_entry(cache: &std::path::Path, key: &str, lines: &[&str]) -> TestResult {
    let digest = sha256_hex(key.as_bytes());
    let file = cache
        .join("_cacache")
        .join("index-v5")
        .join(&digest[..2])
        .join(&digest[2..4])
        .join(&digest[4..]);
    fs::create_dir_all(file.parent().ok_or("no parent")?)?;
    fs::write(file, lines.join("\n"))?;
    Ok(())
}

#[test]
fn npms_cache_index_is_read_by_the_hash_of_the_address() -> TestResult {
    let scratch = Scratch::new("cache-index")?;
    let address = "https://registry.npmjs.org/vsift-cli/-/vsift-cli-0.1.0.tgz";
    let key = format!("make-fetch-happen:request-cache:{address}");
    let entry = |integrity: &str| {
        format!(
            "0123abcd\t{}",
            json!({"key": key, "integrity": integrity, "time": 1, "size": 9})
        )
    };
    cache_entry(scratch.path(), &key, &[&entry(LAUNCHER_INTEGRITY)])?;
    let index = CacheIndex::new(scratch.path().to_path_buf());
    assert_eq!(index.integrity_of(address)?, LAUNCHER_INTEGRITY);

    // The last write wins.
    cache_entry(
        scratch.path(),
        &key,
        &[&entry(OTHER_INTEGRITY), &entry(LAUNCHER_INTEGRITY)],
    )?;
    assert_eq!(index.integrity_of(address)?, LAUNCHER_INTEGRITY);

    // A deleted entry, a weak hash, another key and a missing file are not records.
    let null = format!("0123abcd\t{}", json!({"key": key, "integrity": null}));
    cache_entry(scratch.path(), &key, &[&entry(LAUNCHER_INTEGRITY), &null])?;
    assert!(index.integrity_of(address).is_err());
    cache_entry(scratch.path(), &key, &[&entry("sha1-short")])?;
    assert!(index.integrity_of(address).is_err());
    let other = format!(
        "0123abcd\t{}",
        json!({"key": "another", "integrity": LAUNCHER_INTEGRITY})
    );
    cache_entry(scratch.path(), &key, &[&other])?;
    assert!(index.integrity_of(address).is_err());
    assert!(
        index
            .integrity_of("https://registry.npmjs.org/never/-/never-1.0.0.tgz")
            .is_err()
    );
    Ok(())
}

#[test]
fn only_an_exact_version_can_be_installed() {
    for good in ["0.1.0", "0.2.0-rc.1", "1.0.0", "10.20.30", "0.2.0-beta-2.x"] {
        assert!(validate_version(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "latest",
        "next",
        "^0.1.0",
        "~0.1.0",
        "0.1",
        "0.1.0.0",
        "01.2.3",
        "1.2.3+build",
        "v1.2.3",
        "1.2.3-",
        "1.2.3-a..b",
        "1.2.x",
        ">=0.1.0",
        "0.1.0 ",
        "0.1.0;rm",
    ] {
        assert!(validate_version(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_registry_is_https_or_loopback() {
    for good in [
        "https://registry.npmjs.org/",
        "http://127.0.0.1:4873/",
        "http://localhost:4873/",
    ] {
        assert!(validate_registry(good).is_ok(), "{good}");
    }
    for bad in [
        "http://registry.npmjs.org/",
        "https://registry.npmjs.org",
        "registry.npmjs.org/",
        "https://registry.npmjs.org/ --x",
        "https://r.example/'",
    ] {
        assert!(validate_registry(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_r0_targets_are_the_launchers() {
    assert_eq!(
        target_for("windows", "x86_64").map(|target| (target.package, target.executable)),
        Some(("@vsift/win32-x64", "vsift.exe"))
    );
    assert_eq!(
        target_for("linux", "x86_64").map(|target| target.package),
        Some("@vsift/linux-x64")
    );
    assert_eq!(
        target_for("macos", "aarch64").map(|target| target.package),
        Some("@vsift/darwin-arm64")
    );
    for (os, arch) in [
        ("linux", "aarch64"),
        ("macos", "x86_64"),
        ("windows", "aarch64"),
        ("freebsd", "x86_64"),
    ] {
        assert!(target_for(os, arch).is_none(), "{os} {arch}");
    }
}

#[test]
fn evidence_round_trips_without_losing_a_field() -> TestResult {
    let fabricated = Fabricated::new(false)?;
    let (registry, installed) = consistent(fabricated.target, REGISTRY);
    let proof = fabricated.proof(&registry, &installed, &probe())?;
    let text = serde_json::to_string(&proof.evidence)?;
    let back: InstallEvidence = serde_json::from_str(&text)?;
    assert_eq!(back, proof.evidence);
    Ok(())
}
