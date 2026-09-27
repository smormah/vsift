//! Layer A: power loss at every flush.
//!
//! The device image is rebuilt from the base image (the filesystem as it
//! was when logging began) by replaying the dm-log-writes log up to each
//! replay point. At every point a copy of the image is mounted (ext4 replays
//! its journal, as after a real power loss), the verifier holds the session
//! root to every acknowledgement made before the point, the copy is
//! unmounted and `e2fsck -fn` must find the filesystem clean.

use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use crate::{
    error::CampaignError,
    logwrites::{WriteLog, plan},
    protocol::{Ack, parse_events},
    verify::verify,
    workload::{MAX_ACK_BYTES, read_text, unix_seconds},
};

/// Where the replay runs and what it checks.
#[derive(Clone, Debug)]
pub struct ReplayConfig {
    /// The dm-log-writes log (a file or the log device).
    pub log: PathBuf,
    /// The logged device's image as logging began.
    pub base: PathBuf,
    /// Scratch directory for the rebuilt image and its per-point copy.
    pub work: PathBuf,
    /// The workload's acknowledgement log.
    pub acks: PathBuf,
    /// Where each copy is mounted.
    pub mount_point: PathBuf,
    /// The session root inside the mounted filesystem.
    pub root_in_filesystem: PathBuf,
    /// The report file (one line per failing point, then a summary).
    pub report: PathBuf,
    /// Check only every `stride`-th point (1: every point).
    pub stride: usize,
}

/// What the replay found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplaySummary {
    /// Log entries.
    pub entries: usize,
    /// Replay points checked.
    pub points: usize,
    /// Acknowledgements in the log.
    pub acks: usize,
    /// Points where at least one acknowledgement was lost.
    pub lost_points: usize,
    /// Distinct acknowledgements lost at some point.
    pub lost_acks: usize,
    /// Points with damage.
    pub damaged_points: usize,
    /// Points where `e2fsck -fn` did not report a clean filesystem.
    pub fsck_failures: usize,
    /// Points where the copy could not be mounted.
    pub mount_failures: usize,
}

impl ReplaySummary {
    /// The summary line.
    #[must_use]
    pub fn line(&self, seconds: u64) -> String {
        format!(
            "SUMMARY entries={} points={} acks={} lost_points={} lost_acks={} damaged_points={} \
             fsck_failures={} mount_failures={} seconds={seconds}",
            self.entries,
            self.points,
            self.acks,
            self.lost_points,
            self.lost_acks,
            self.damaged_points,
            self.fsck_failures,
            self.mount_failures
        )
    }
}

fn run(
    program: &'static str,
    path: &str,
    arguments: &[&std::ffi::OsStr],
) -> Result<i32, CampaignError> {
    let status = Command::new(path)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| CampaignError::Command {
            program,
            status: None,
        })?;
    status.code().ok_or(CampaignError::Command {
        program,
        status: None,
    })
}

fn require(
    program: &'static str,
    path: &str,
    arguments: &[&std::ffi::OsStr],
) -> Result<(), CampaignError> {
    match run(program, path, arguments)? {
        0 => Ok(()),
        code => Err(CampaignError::Command {
            program,
            status: Some(code),
        }),
    }
}

/// Replays the log point by point and verifies each point.
///
/// # Errors
///
/// A [`CampaignError`] when the harness fails; findings are in the summary
/// and the report.
pub fn replay(config: &ReplayConfig) -> Result<ReplaySummary, CampaignError> {
    let started = Instant::now();
    let events = parse_events(&read_text(&config.acks, MAX_ACK_BYTES)?);
    if !events.malformed.is_empty() {
        return Err(CampaignError::MalformedAcks(events.malformed));
    }
    let acks: Vec<Ack> = events.acks;
    let mut log_file =
        File::open(&config.log).map_err(CampaignError::io("opening the write log", &config.log))?;
    let length = log_file
        .seek(SeekFrom::End(0))
        .map_err(CampaignError::io("sizing the write log", &config.log))?;
    let log = WriteLog::read(&mut log_file, length)?;
    let sequence: Vec<u64> = acks.iter().map(|ack| ack.seq).collect();
    let points = plan(&log, &sequence).map_err(CampaignError::MissingMark)?;
    let image = config.work.join("replay.img");
    let copy = config.work.join("point.img");
    require(
        "cp",
        "/usr/bin/cp",
        &[
            "--sparse=always".as_ref(),
            config.base.as_os_str(),
            image.as_os_str(),
        ],
    )?;
    let mut target = OpenOptions::new()
        .write(true)
        .open(&image)
        .map_err(CampaignError::io("opening the replay image", &image))?;
    let mut report = File::create(&config.report)
        .map_err(CampaignError::io("creating the report", &config.report))?;
    let mut summary = ReplaySummary {
        entries: log.entries.len(),
        acks: acks.len(),
        ..ReplaySummary::default()
    };
    let mut lost = std::collections::BTreeSet::new();
    let mut applied = 0;
    let stride = config.stride.max(1);
    let final_point = points.len().saturating_sub(1);
    for (number, point) in points.iter().enumerate() {
        log.apply(&mut log_file, &mut target, applied..point.replayed)?;
        applied = point.replayed;
        if number % stride != 0 && number != final_point {
            continue;
        }
        target
            .flush()
            .map_err(CampaignError::io("flushing the replay image", &image))?;
        summary.points += 1;
        let required = acks.get(..point.required).unwrap_or(&acks);
        let line = check_point(config, &image, &copy, required, &mut summary, &mut lost)?;
        if let Some(line) = line {
            writeln!(
                report,
                "POINT {number} replayed={} required={} {line}",
                point.replayed, point.required
            )
            .map_err(CampaignError::io("writing the report", &config.report))?;
        }
    }
    summary.lost_acks = lost.len();
    writeln!(report, "{}", summary.line(started.elapsed().as_secs()))
        .map_err(CampaignError::io("writing the report", &config.report))?;
    Ok(summary)
}

/// Mounts a copy of the image as it is now, verifies it, unmounts it and
/// checks it; returns a report line when anything is wrong.
fn check_point(
    config: &ReplayConfig,
    image: &Path,
    copy: &Path,
    required: &[Ack],
    summary: &mut ReplaySummary,
    lost: &mut std::collections::BTreeSet<u64>,
) -> Result<Option<String>, CampaignError> {
    require(
        "cp",
        "/usr/bin/cp",
        &[
            "--sparse=always".as_ref(),
            image.as_os_str(),
            copy.as_os_str(),
        ],
    )?;
    let mounted = run(
        "mount",
        "/usr/bin/mount",
        &[
            "-t".as_ref(),
            "ext4".as_ref(),
            "-o".as_ref(),
            "loop".as_ref(),
            copy.as_os_str(),
            config.mount_point.as_os_str(),
        ],
    )?;
    if mounted != 0 {
        summary.mount_failures += 1;
        return Ok(Some(format!("mount-failed status={mounted}")));
    }
    let findings = verify(
        &config.mount_point.join(&config.root_in_filesystem),
        required,
        unix_seconds()?,
    );
    require(
        "umount",
        "/usr/bin/umount",
        &[config.mount_point.as_os_str()],
    )?;
    let fsck = run(
        "e2fsck",
        "/usr/sbin/e2fsck",
        &["-fn".as_ref(), copy.as_os_str()],
    )?;
    if fsck != 0 {
        summary.fsck_failures += 1;
    }
    if !findings.lost.is_empty() {
        summary.lost_points += 1;
        lost.extend(findings.lost.iter().map(|(seq, _)| *seq));
    }
    if !findings.damage.is_empty() {
        summary.damaged_points += 1;
    }
    if findings.clean() && fsck == 0 {
        return Ok(None);
    }
    let details: Vec<String> = findings.lines("VERIFY").into_iter().take(8).collect();
    Ok(Some(format!("fsck={fsck} {}", details.join(" | "))))
}
