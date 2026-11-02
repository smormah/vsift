//! Opt-in, live: `ingest --url` against public-domain NASA video on the
//! internet (ADR 0028, #407). Never part of the ordinary tests; run by the
//! manual hosted workflow `Online links live` only, in a release build (a
//! development build resolves no name). The items are the real-media set's
//! RM09 (`images-assets.nasa.gov`, direct) and RM06 (the Internet Archive,
//! through its redirects to a storage node), each pinned by size and SHA-256
//! in `tools/p14-campaigns/real-media/manifest.json`, so the session's source
//! identity must be the pinned digest; RM09's NASA page, which is a web page
//! and must be refused as one with no yt-dlp set up; and RM06's Internet Archive
//! page, fetched through a real managed yt-dlp and Deno (ADR 0028 section 4.9,
//! #409), which this test installs into its own folder. On Windows and macOS
//! that page goes through the pinned yt-dlp the workflow fetches and registers
//! instead, the only way pages work there. One NASA video on a video site is a
//! separate, dispatch-only check, skipped below. The opt-in platform
//! qualification (#409) is another: it reads the named list of
//! `docs/operations/online-video-qualification.md` and keeps a record.
//!
//! ```console
//! cargo test --release --locked -p vsift-cli --test link_live -- --ignored --skip a_nasa_youtube_page --skip the_listed_videos --nocapture
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

const RM06_PAGE: &str = "https://archive.org/details/jsc2023m000168_NTV_VF_Astronauts_Discuss_Record_Breaking_Mission_230912-MP4.mp4";

/// NASA's own upload of Perseverance's landing, on its official channel.
const NASA_YOUTUBE_PAGE: &str = "https://www.youtube.com/watch?v=4czjS9h4Fpg";

/// The pinned publisher file for this system (ADR 0028 section 4.1).
#[cfg(windows)]
const PINNED_USER_YT_DLP_SHA256: &str =
    "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a";
#[cfg(target_os = "macos")]
const PINNED_USER_YT_DLP_SHA256: &str =
    "0f192b7ec147ab6288885d6351d9ab67367640029b4377576ef46dd79cf7b202";

/// One page of the qualification: the named list's id, its link (public, with
/// no token, so naming it is safe), its licence as the page states it, and,
/// for a refusal case, the one reason it must give.
struct Listed {
    id: &'static str,
    link: &'static str,
    licence: &'static str,
    must_be_refused_as: Option<&'static str>,
}

/// `docs/operations/online-video-qualification.md`, Q1 to Q6 and R1. The
/// pages are not asserted to succeed: a bot wall, a login wall or a site
/// change is a result to record. Only a pair joined counts as the real pair.
const LISTED: [Listed; 7] = [
    Listed {
        id: "Q1",
        link: "https://www.facebook.com/NASA/videos/438313177583525/",
        licence: "public domain (US government work); the site's terms apply",
        must_be_refused_as: None,
    },
    Listed {
        id: "Q2",
        link: "https://www.facebook.com/NASA/videos/645832639583660/",
        licence: "public domain (US government work); the site's terms apply",
        must_be_refused_as: None,
    },
    Listed {
        id: "Q3",
        link: "https://video.blender.org/videos/watch/ac71efff-5e26-440b-8c38-dd1f5b598e02",
        licence: "Attribution (CC BY), as the instance states",
        must_be_refused_as: None,
    },
    Listed {
        id: "Q4",
        link: "https://media.ccc.de/v/matrix-conference-2026-99063-a-year-and-a-half-of-continuwuity",
        licence: "CC BY-SA 4.0, as the site states",
        must_be_refused_as: None,
    },
    Listed {
        id: "Q5",
        link: "https://commons.wikimedia.org/wiki/File:Editing_basics_-_watchlists.webm",
        licence: "CC BY-SA 3.0, as the file's page states",
        must_be_refused_as: None,
    },
    Listed {
        id: "Q6",
        link: "https://www.nasa.gov/video-detail/nhq20230118armd01",
        licence: "public domain (US government work)",
        must_be_refused_as: None,
    },
    Listed {
        id: "R1",
        link: "https://video.blender.org/w/p/cQ4v4uoPbLdnhFrqsEx69m",
        licence: "Attribution (CC BY), as the instance states; a playlist page",
        must_be_refused_as: Some("playlist"),
    },
];

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
    let (code, stdout) = vsift_bytes(root, arguments)?;
    Ok((code, serde_json::from_slice(&stdout)?))
}

fn vsift_bytes(
    root: &OwnedRoot,
    arguments: &[&str],
) -> Result<(Option<i32>, Vec<u8>), Box<dyn Error>> {
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
    Ok((output.status.code(), output.stdout))
}

/// Installs the core and the `online_links` group into this root's own
/// folder, as a person does after saying yes.
fn install_online_links(root: &OwnedRoot) -> TestResult {
    let plan_path = root.0.join("plan.json");
    let (code, plan) = vsift_bytes(
        root,
        &[
            "setup",
            "plan",
            "--profile",
            "desktop",
            "--include",
            "online_links",
        ],
    )?;
    assert_eq!(code, Some(0), "setup plan failed");
    let digest = serde_json::from_slice::<Value>(&plan)?["data"]["plan_digest"]
        .as_str()
        .ok_or("the plan has no digest")?
        .to_owned();
    fs::write(&plan_path, &plan)?;
    let (code, installed) = vsift(
        root,
        &[
            "setup",
            "install",
            "--plan",
            plan_path.to_str().ok_or("plan path")?,
            "--accept-plan",
            &digest,
        ],
    )?;
    assert_eq!(code, Some(0), "{installed}");
    Ok(())
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

/// The whole path against the real site: the web page answer, the managed
/// pair, phase 1's checks, the download and the record.
#[test]
#[ignore = "opt-in live check; installs the real yt-dlp and reads a public Internet Archive page; the manual hosted workflow runs it"]
fn a_public_page_is_fetched_through_the_managed_yt_dlp() -> TestResult {
    let root = OwnedRoot::new()?;
    install_online_links(&root)?;
    let (code, opened) = vsift(&root, &["ingest", "--url", RM06_PAGE])?;
    println!(
        "RM06 page: {}",
        opened["data"].get("origin").unwrap_or(&opened["error"])
    );
    assert_eq!(code, Some(0), "{opened}");
    let origin = &opened["data"]["origin"];
    assert_eq!(origin["kind"], "platform_page");
    assert_eq!(origin["yt_dlp"]["source"], "managed");
    assert_eq!(origin["js_runtime"]["source"], "managed");
    assert!(origin.get("address_check").is_none());
    let text = opened.to_string();
    assert!(
        !text.contains("/details/") && !text.contains("Astronauts"),
        "the link or the title was printed"
    );
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or("no session id")?;
    let (code, _) = vsift(&root, &["session", "close", session])?;
    assert_eq!(code, Some(0));
    Ok(())
}

/// The engine's whole path through a registered yt-dlp: found, identified as
/// `setup check` identifies it, run with no runtime, and recorded. The
/// workflow fetches the pinned file; the recorded digest is the proof of it.
#[cfg(any(windows, target_os = "macos"))]
#[test]
#[ignore = "opt-in live check; set VSIFT_LIVE_USER_YT_DLP to the pinned yt-dlp and it reads a public Internet Archive page; the manual hosted workflow runs it"]
fn a_public_page_is_fetched_through_a_registered_yt_dlp() -> TestResult {
    let supplied =
        env::var("VSIFT_LIVE_USER_YT_DLP").map_err(|_| "set VSIFT_LIVE_USER_YT_DLP to the file")?;
    let root = OwnedRoot::new()?;
    let (code, configured) = vsift(
        &root,
        &["setup", "configure", "yt-dlp", "--executable", &supplied],
    )?;
    assert_eq!(code, Some(0), "{configured}");
    let (code, opened) = vsift(&root, &["ingest", "--url", RM06_PAGE])?;
    println!(
        "RM06 page: {}",
        opened["data"].get("origin").unwrap_or(&opened["error"])
    );
    assert_eq!(code, Some(0), "{opened}");
    let origin = &opened["data"]["origin"];
    assert_eq!(origin["kind"], "platform_page");
    assert_eq!(origin["yt_dlp"]["source"], "user_managed");
    assert_eq!(origin["yt_dlp"]["sha256"], PINNED_USER_YT_DLP_SHA256);
    assert!(
        origin.get("js_runtime").is_none(),
        "no runtime was registered: {origin}"
    );
    let text = opened.to_string();
    assert!(
        !text.contains("/details/") && !text.contains("Astronauts"),
        "the link or the title was printed"
    );
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or("no session id")?;
    let (code, _) = vsift(&root, &["session", "close", session])?;
    assert_eq!(code, Some(0));
    Ok(())
}

/// A hosted runner may or may not meet a bot wall on that site, so two outcomes
/// pass: the typed refusal, or real separate streams joined. Anything else
/// fails, and goes to the planner with the evidence.
#[test]
#[ignore = "opt-in live check; a dispatch of the manual hosted workflow runs it once for a change and at each re-pin"]
fn a_nasa_youtube_page_is_joined_or_refused_as_a_bot_check() -> TestResult {
    let root = OwnedRoot::new()?;
    install_online_links(&root)?;
    let (code, opened) = vsift(&root, &["ingest", "--url", NASA_YOUTUBE_PAGE])?;
    println!(
        "YouTube page: {}",
        opened["data"].get("origin").unwrap_or(&opened["error"])
    );
    let text = opened.to_string();
    assert!(
        !text.contains("watch?v=") && !text.contains("Perseverance"),
        "the link or the title was printed"
    );
    if code != Some(0) {
        assert_eq!(opened["error"]["code"], "SOURCE_FETCH_FAILED", "{opened}");
        assert_eq!(opened["error"]["reason"], "bot_check", "{opened}");
        return Ok(());
    }
    let origin = &opened["data"]["origin"];
    assert_eq!(origin["kind"], "platform_page");
    assert_eq!(origin["joined"], "audio_video", "{origin}");
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or("no session id")?;
    let (code, _) = vsift(&root, &["session", "close", session])?;
    assert_eq!(code, Some(0));
    Ok(())
}

/// For the one run from an ordinary network that the hosted runners cannot
/// give (they meet `YouTube`'s bot wall): the user-supplied tools, found through
/// `setup configure`, and no pass on a refusal. Only `joined: audio_video`
/// passes, because `a_nasa_youtube_page_is_joined_or_refused_as_a_bot_check`
/// already passes on the typed refusal and so proves nothing about a pair.
/// `VSIFT_LIVE_USER_YT_DLP` is the pinned yt-dlp; `VSIFT_LIVE_USER_JS_RUNTIME`
/// is a Deno the person supplies, which `YouTube` needs.
#[test]
#[ignore = "opt-in live check; run by hand from an ordinary network with the maintainer's go, never on this machine's automatic runs"]
fn a_nasa_youtube_page_is_joined_through_registered_tools() -> TestResult {
    let tool = env::var("VSIFT_LIVE_USER_YT_DLP").map_err(|_| "set VSIFT_LIVE_USER_YT_DLP")?;
    let root = OwnedRoot::new()?;
    let (code, configured) = vsift(
        &root,
        &["setup", "configure", "yt-dlp", "--executable", &tool],
    )?;
    assert_eq!(code, Some(0), "{configured}");
    if let Ok(runtime) = env::var("VSIFT_LIVE_USER_JS_RUNTIME") {
        let (code, configured) = vsift(
            &root,
            &["setup", "configure", "js-runtime", "--executable", &runtime],
        )?;
        assert_eq!(code, Some(0), "{configured}");
    }
    let (code, opened) = vsift(&root, &["ingest", "--url", NASA_YOUTUBE_PAGE])?;
    println!(
        "YouTube page: {}",
        opened["data"].get("origin").unwrap_or(&opened["error"])
    );
    assert_eq!(code, Some(0), "no pair was joined: {}", opened["error"]);
    let origin = &opened["data"]["origin"];
    assert_eq!(origin["joined"], "audio_video", "{origin}");
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or("no session id")?;
    let (code, _) = vsift(&root, &["session", "close", session])?;
    assert_eq!(code, Some(0));
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

/// The version of each installed optional component, as `setup check` reports
/// it, for the record.
fn installed_versions(root: &OwnedRoot) -> Result<Value, Box<dyn Error>> {
    let (code, checked) = vsift(root, &["setup", "check", "--profile", "desktop"])?;
    assert_eq!(code, Some(0), "{checked}");
    let components = checked["optional_components"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["name"] == "online_links"))
        .ok_or("no online_links row")?["components"]
        .clone();
    assert!(
        components.as_array().is_some_and(|list| !list.is_empty()),
        "no optional component is reported: {checked}"
    );
    Ok(components)
}

/// What one listed page did: fetched (and how it was joined) or refused (and
/// why). Anything but a fetch or a typed `SOURCE_FETCH_FAILED` is a failure
/// of `VSift`'s own, and goes in `problems`.
fn qualify_one(
    root: &OwnedRoot,
    item: &Listed,
    problems: &mut Vec<String>,
) -> Result<Value, Box<dyn Error>> {
    let mut entry = serde_json::json!({
        "id": item.id,
        "licence": item.licence,
        "host": item.link.split('/').nth(2).unwrap_or(""),
    });
    // An answer that is not JSON must not lose the record of the other pages.
    let Ok((code, outcome)) = vsift(root, &["ingest", "--url", item.link]) else {
        entry["outcome"] = "unreadable".into();
        problems.push(format!("{}: the answer was not JSON", item.id));
        println!("{} {entry}", item.id);
        return Ok(entry);
    };
    let text = outcome.to_string();
    if text.contains("https://") {
        problems.push(format!("{}: a link was printed", item.id));
    }
    if code == Some(0) {
        let data = &outcome["data"];
        entry["outcome"] = "fetched".into();
        entry["kind"] = data["origin"]["kind"].clone();
        entry["provider"] = data["origin"]["provider"].clone();
        entry["joined"] = data["origin"]["joined"].clone();
        entry["bytes"] = data["source_bytes"].clone();
        if let Some(expected) = item.must_be_refused_as {
            problems.push(format!(
                "{}: fetched, but must be refused as {expected}",
                item.id
            ));
        }
        if let Some(session) = data["session_id"].as_str() {
            let (closed, _) = vsift(root, &["session", "close", session])?;
            assert_eq!(closed, Some(0));
        }
    } else if outcome["error"]["code"] == "SOURCE_FETCH_FAILED" {
        let reason = outcome["error"]["reason"].as_str().unwrap_or("none");
        entry["outcome"] = "refused".into();
        entry["reason"] = reason.into();
        if item
            .must_be_refused_as
            .is_some_and(|expected| expected != reason)
        {
            problems.push(format!("{}: refused as {reason}", item.id));
        }
    } else {
        entry["outcome"] = "untyped".into();
        entry["code"] = outcome["error"]["code"].clone();
        problems.push(format!(
            "{}: an untyped failure: {}",
            item.id, outcome["error"]["code"]
        ));
    }
    println!("{} {entry}", item.id);
    Ok(entry)
}

/// #409's opt-in platform qualification: every page of the named list, through
/// the managed pair, from a hosted runner. The record is the product: yt-dlp
/// and runtime versions, the date, the outcome of each, and whether a real pair
/// of streams was ever joined. It holds no link, and the test fails only when
/// `VSift` itself misbehaves (a link printed, an untyped failure, a refusal case
/// that gave another reason), never because a site walled the runner.
#[test]
#[ignore = "opt-in live check; installs the real yt-dlp and reads the named public pages; the manual hosted workflow runs it with `qualify`"]
fn the_listed_videos_are_fetched_or_refused_and_recorded() -> TestResult {
    let root = OwnedRoot::new()?;
    install_online_links(&root)?;
    let versions = installed_versions(&root)?;
    let mut problems = Vec::new();
    let mut outcomes = Vec::new();
    for item in &LISTED {
        outcomes.push(qualify_one(&root, item, &mut problems)?);
    }
    let pair_seen = outcomes
        .iter()
        .any(|entry| entry["joined"] == "audio_video");
    let record = serde_json::json!({
        "record_version": 1,
        "date": env::var("VSIFT_QUALIFICATION_DATE").unwrap_or_else(|_| String::from("unknown")),
        "runner": env::consts::OS,
        "components": versions,
        "outcomes": outcomes,
        "pair_seen": pair_seen,
        "problems": problems,
    });
    let text = serde_json::to_string_pretty(&record)?;
    println!("{text}");
    if let Ok(path) = env::var("VSIFT_QUALIFICATION_RECORD") {
        fs::write(
            path,
            format!(
                "{text}
"
            ),
        )?;
    }
    assert!(problems.is_empty(), "{problems:?}");
    Ok(())
}
