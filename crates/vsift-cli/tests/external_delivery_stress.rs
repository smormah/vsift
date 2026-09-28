//! The repeated external-delivery simulation (P11 PR 3, ADR 0021 section
//! 4; the P11 packet gate): an external queue that delivers each request at
//! least once, kills workers at random, delivers again and sometimes twice
//! at once, until every request has completed.
//!
//! Twenty operation ids, each an ingest of its own source, a retained bundle
//! and a close, run through the release-shaped binary in one ephemeral
//! workspace. Every round starts one `job run` per unfinished request, a
//! concurrent duplicate for some, and kills some of them after a random
//! delay (`SIGKILL` on Unix, `TerminateProcess` on Windows). A process that
//! was not killed must answer `complete` (fresh or replayed) or, for a
//! duplicate, `BUSY` with the 2 s hint; nothing else. Once every request has
//! completed, each is delivered again and must replay exactly the result it
//! first recorded (identical result digest), and the workspace must hold
//! exactly one opened session per operation id.
//!
//! Opt-in, because it starts hundreds of processes:
//!
//! `cargo test -p vsift-cli --locked --test external_delivery_stress -- --ignored --nocapture`
//!
//! `VSIFT_STRESS_SEED` fixes the random choices (the seed used is printed).

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use vsift_domain::SessionId;
use vsift_infrastructure::FilesystemSessionStore;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-external-delivery-";
const KEYS: u32 = 20;
const MAX_ROUNDS: u32 = 60;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct Layout(PathBuf);

impl Layout {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        let layout = Self(path);
        fs::create_dir_all(layout.path("inputs"))?;
        fs::create_dir(layout.path("bundles"))?;
        Ok(layout)
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }

    fn command(&self) -> Command {
        let base = self.path("user");
        let mut command = Command::new(assert_cmd::cargo::cargo_bin("vsift"));
        command
            .env("LOCALAPPDATA", &base)
            .env("XDG_CONFIG_HOME", &base)
            .env("XDG_CACHE_HOME", &base)
            .env("HOME", &base)
            .arg("--session-root")
            .arg(self.path("workspace"))
            .stdin(Stdio::null());
        command
    }

    fn deliver(&self, request: &Path) -> Built<Child> {
        Ok(self
            .command()
            .args(["job", "run", "--request"])
            .arg(request)
            .arg("--input-root")
            .arg(self.path("inputs"))
            .arg("--bundle-root")
            .arg(self.path("bundles"))
            .args(["--admission-wait-ms", "5000", "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?)
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// A small deterministic generator (xorshift64*), so a failing run can be
/// repeated with its seed.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound.max(1)
    }
}

fn operation(key: u32) -> String {
    format!("op_{key:032x}")
}

/// A result's data with the one member a replay changes set as recorded.
fn recorded(data: &Value) -> Value {
    let mut data = data.clone();
    data["replayed"] = Value::Bool(false);
    data
}

/// One delivery of a request: its process, and whether it will be killed.
struct Delivery {
    key: u32,
    child: Child,
    kill_after: Option<Duration>,
    started: Instant,
}

#[test]
#[ignore = "opt-in stress: hundreds of worker processes, killed at random"]
#[allow(
    clippy::too_many_lines,
    reason = "One simulation: its rounds and its final checks read together"
)]
fn repeated_external_delivery_commits_once() -> TestResult {
    let seed = env::var("VSIFT_STRESS_SEED")
        .ok()
        .and_then(|seed| seed.parse::<u64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(1, |elapsed| u64::from(elapsed.subsec_nanos()) | 1)
        });
    println!("VSIFT_STRESS_SEED={seed}");
    let mut random = Random(seed);
    let layout = Layout::new()?;
    let initialised = layout
        .command()
        .args([
            "session",
            "init-workspace",
            "--durability",
            "ephemeral",
            "--admission-slots",
            "8",
            "--json",
        ])
        .output()?;
    assert_eq!(initialised.status.code(), Some(0), "{initialised:?}");

    let mut requests = BTreeMap::new();
    for key in 0..KEYS {
        fs::write(
            layout.path("inputs").join(format!("source-{key}.mp4")),
            format!("\0\0\0\x18ftypisomexternal-delivery-{key}"),
        )?;
        let file = layout.path(&format!("request-{key}.json"));
        fs::write(
            &file,
            format!(
                r#"{{"schema_version":"1","operation_id":"{}","durability":"ephemeral","target":{{"ingest":{{"source":"source-{key}.mp4","transcript":null}}}},"steps":[{{"retain":{{"bundle_name":"key-{key}","include_source":false}}}},{{"close":{{}}}}]}}"#,
                operation(key)
            ),
        )?;
        requests.insert(key, file);
    }

    let mut completed: BTreeMap<u32, Value> = BTreeMap::new();
    let mut kills = 0_u32;
    let mut busy = 0_u32;
    let mut replays = 0_u32;
    let mut rounds = 0_u32;
    while completed.len() < requests.len() {
        rounds += 1;
        assert!(rounds <= MAX_ROUNDS, "not every request completed");
        let mut deliveries = Vec::new();
        for (key, file) in &requests {
            if completed.contains_key(key) && random.below(4) != 0 {
                continue;
            }
            let copies = if random.below(3) == 0 { 2 } else { 1 };
            for _ in 0..copies {
                deliveries.push(Delivery {
                    key: *key,
                    child: layout.deliver(file)?,
                    kill_after: (random.below(2) == 0)
                        .then(|| Duration::from_millis(random.below(400))),
                    started: Instant::now(),
                });
            }
        }
        // Kill the chosen ones at their moment, then collect everyone.
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut pending: Vec<Delivery> = deliveries;
        while !pending.is_empty() {
            assert!(Instant::now() < deadline, "a delivery never ended");
            let mut still = Vec::new();
            for mut delivery in pending {
                if let Some(status) = delivery.child.try_wait()? {
                    let mut stdout = Vec::new();
                    if let Some(mut out) = delivery.child.stdout.take() {
                        std::io::Read::read_to_end(&mut out, &mut stdout)?;
                    }
                    let value: Value = serde_json::from_slice(&stdout)
                        .map_err(|error| format!("key {}: {error}: {status}", delivery.key))?;
                    match status.code() {
                        Some(0) => {
                            assert_eq!(value["status"], "complete", "{value}");
                            let data = recorded(&value["data"]);
                            if value["data"]["replayed"] == true {
                                replays += 1;
                            }
                            match completed.get(&delivery.key) {
                                Some(first) => assert_eq!(first, &data, "key {}", delivery.key),
                                None => {
                                    completed.insert(delivery.key, data);
                                }
                            }
                        }
                        Some(4) => {
                            assert_eq!(value["error"]["code"], "BUSY", "{value}");
                            assert_eq!(value["error"]["retry_after_ms"], 2_000);
                            busy += 1;
                        }
                        other => {
                            return Err(format!(
                                "key {}: unexpected exit {other:?}: {value}",
                                delivery.key
                            )
                            .into());
                        }
                    }
                    continue;
                }
                if let Some(after) = delivery.kill_after
                    && delivery.started.elapsed() >= after
                {
                    // A kill after the process ended on its own is harmless.
                    let _ = delivery.child.kill();
                    let _ = delivery.child.wait();
                    kills += 1;
                    continue;
                }
                still.push(delivery);
            }
            pending = still;
            thread::sleep(Duration::from_millis(5));
        }
    }

    // Every later delivery replays exactly the first recorded result.
    let mut sessions = BTreeSet::new();
    for (key, file) in &requests {
        let output = layout.deliver(file)?.wait_with_output()?;
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(value["data"]["replayed"], true);
        assert_eq!(completed.get(key), Some(&recorded(&value["data"])));
        let session = value["data"]["session_id"]
            .as_str()
            .ok_or("no session")?
            .to_owned();
        assert!(sessions.insert(session), "two requests share a session");
        assert!(
            layout
                .path("bundles")
                .join(format!("key-{key}"))
                .join("bundle.json")
                .exists()
        );
    }

    // Exactly one opened session per operation id: a killed ingest leaves
    // at most a registration that never opened.
    let store = FilesystemSessionStore::open_existing(layout.path("workspace"))?;
    let mut opened = BTreeSet::new();
    for entry in fs::read_dir(layout.path("workspace").join("sessions"))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        // A killed initialisation can leave its staging entry beside the
        // sessions; only session directories count.
        let Ok(session) = SessionId::parse(name) else {
            continue;
        };
        if store.indexed_session_status(&session)?.is_some() {
            opened.insert(session.as_str().to_owned());
        }
    }
    assert_eq!(opened, sessions);
    println!(
        "{} requests, {rounds} rounds, {kills} kills, {busy} busy duplicates, {replays} replays",
        requests.len()
    );
    Ok(())
}
