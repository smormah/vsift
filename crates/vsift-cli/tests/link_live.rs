//! Opt-in, live: `ingest --url` against public-domain NASA video on the
//! internet (ADR 0028, #407). Never part of the ordinary tests; run by the
//! manual hosted workflow `Online links live` only, in a release build (a
//! development build resolves no name). The items are the real-media set's
//! RM09 (`images-assets.nasa.gov`, direct) and RM06 (the Internet Archive,
//! through its redirects to a storage node), each pinned by size and SHA-256
//! in `tools/p14-campaigns/real-media/manifest.json`, so the session's source
//! identity must be the pinned digest; and RM09's NASA page, which is a web
//! page and must be refused as one.
//!
//! ```console
//! cargo test --release --locked -p vsift-cli --test link_live -- --ignored --nocapture
//! ```

use std::{
    env,
    error::Error,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-link-live-";

/// A pinned public-domain file: its link, size and SHA-256, and the host
/// its bytes come from.
struct Pinned {
    id: &'static str,
    link: &'static str,
    bytes: u64,
    sha256: &'static str,
    final_host_suffix: &'static str,
}

const RM09: Pinned = Pinned {
    id: "RM09",
    link: "https://images-assets.nasa.gov/video/KSC-20240225-MH-JBS01-0001-CCP_SpaceX_Crew_8_Arrival_KSC-M4585/KSC-20240225-MH-JBS01-0001-CCP_SpaceX_Crew_8_Arrival_KSC-M4585~mobile.mp4",
    bytes: 72_873_726,
    sha256: "ee5501f1e25c8240a8e2dae61eebabf9446615d4c3558c3efbb098a1cc4bf41a",
    final_host_suffix: "images-assets.nasa.gov",
};

const RM06: Pinned = Pinned {
    id: "RM06",
    link: "https://archive.org/download/jsc2023m000168_NTV_VF_Astronauts_Discuss_Record_Breaking_Mission_230912-MP4.mp4/jsc2023m000168_NTV_VF_Astronauts_Discuss_Record_Breaking_Mission_230912-MP4.ia.mp4",
    bytes: 60_378_293,
    sha256: "a48781d5ce3496f908429ae3006a090b8fb6159cafaee94a985d6dd2f64933ca",
    final_host_suffix: "archive.org",
};

const RM09_PAGE: &str = "https://images.nasa.gov/details/KSC-20240225-MH-JBS01-0001-CCP_SpaceX_Crew_8_Arrival_KSC-M4585";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for OwnedRoot {
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

fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Result<(Option<i32>, Value), Box<dyn Error>> {
    let base = root.0.join("user");
    let output = Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("XDG_CACHE_HOME", &base)
        .env("HOME", &base)
        .arg("--session-root")
        .arg(root.0.join("sessions"))
        .args(arguments)
        .arg("--json")
        .timeout(Duration::from_mins(20))
        .output()?;
    Ok((
        output.status.code(),
        serde_json::from_slice(&output.stdout)?,
    ))
}

fn fetch_pinned(item: &Pinned) -> TestResult {
    let root = OwnedRoot::new()?;
    let (code, opened) = vsift(&root, &["ingest", "--url", item.link])?;
    println!(
        "{}: {}",
        item.id,
        opened["data"].get("origin").unwrap_or(&opened["error"])
    );
    assert_eq!(code, Some(0), "{}: {opened}", item.id);
    let data = &opened["data"];
    assert_eq!(
        data["source_id"],
        format!("src_sha256_{}", item.sha256),
        "{}",
        item.id
    );
    assert_eq!(data["source_bytes"], item.bytes);
    let origin = &data["origin"];
    assert_eq!(origin["kind"], "direct_https");
    assert_eq!(origin["bytes"], item.bytes);
    assert_eq!(origin["address_check"], "direct");
    assert!(
        origin["final_host"]
            .as_str()
            .is_some_and(|host| host.ends_with(item.final_host_suffix)),
        "{}: {origin}",
        item.id
    );
    let text = opened.to_string();
    assert!(
        !text.contains("/download/") && !text.contains("/video/"),
        "the link's path was printed"
    );
    let session = data["session_id"].as_str().ok_or("no session id")?;
    let (code, _) = vsift(&root, &["session", "close", session])?;
    assert_eq!(code, Some(0));
    Ok(())
}

#[test]
#[ignore = "opt-in live check; fetches public-domain NASA video from the internet; the manual hosted workflow runs it"]
fn pinned_public_domain_videos_are_fetched_into_sessions_as_their_pinned_bytes() -> TestResult {
    fetch_pinned(&RM09)?;
    fetch_pinned(&RM06)?;
    Ok(())
}

#[test]
#[ignore = "opt-in live check; reads a public NASA web page; the manual hosted workflow runs it"]
fn a_public_video_page_is_refused_as_a_web_page() -> TestResult {
    let root = OwnedRoot::new()?;
    let (code, refused) = vsift(&root, &["ingest", "--url", RM09_PAGE])?;
    println!("page: {}", refused["error"]);
    assert_eq!(code, Some(3), "{refused}");
    assert_eq!(refused["error"]["code"], "SOURCE_FETCH_FAILED");
    assert_eq!(refused["error"]["reason"], "web_page");
    Ok(())
}
