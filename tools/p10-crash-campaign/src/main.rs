//! The Ubuntu 24.04 / ext4 durable-publication crash campaign (P10 PR 4,
//! ADR 0020 section 7).
//!
//! One binary plays three parts; the scripts next to it (`scripts/`) set up
//! the devices and machines around it:
//!
//! - `workload` runs durable operations and acknowledges each success;
//! - `verify` holds a session root to the acknowledgements made before a
//!   crash (layer B inside the rebooted guest, layer C after a write error);
//! - `replay` rebuilds the device at every flush of a dm-log-writes log and
//!   verifies each point (layer A).
//!
//! Exit status: 0 when the step ran and found nothing wrong, 1 when it found
//! lost acknowledgements or damage, 2 when the harness itself failed.

mod assess;
mod error;
mod logwrites;
mod protocol;
mod replay;
mod rng;
mod standins;
mod verify;
mod workload;

use std::{fs::OpenOptions, io::Write as _, path::PathBuf, process::ExitCode, time::Duration};

use clap::{Parser, Subcommand};

use crate::{
    error::CampaignError,
    protocol::parse_events,
    replay::ReplayConfig,
    workload::{MAX_ACK_BYTES, Mix, WorkloadConfig, read_text, unix_seconds},
};

/// The P10 crash campaign harness.
#[derive(Debug, Parser)]
#[command(name = "vsift-crash-campaign", version)]
struct Arguments {
    #[command(subcommand)]
    step: Step,
}

#[derive(Debug, Subcommand)]
enum Step {
    /// Runs durable operations and acknowledges each success.
    Workload {
        /// Session root on the filesystem under test.
        #[arg(long)]
        root: PathBuf,
        /// Scratch directory off the filesystem under test.
        #[arg(long)]
        scratch: PathBuf,
        /// Acknowledgement channel (a file, or a serial port with --drain).
        #[arg(long)]
        ack_out: PathBuf,
        /// Wait for the serial port to transmit every line.
        #[arg(long)]
        drain: bool,
        /// dm-log-writes device to mark after every acknowledgement.
        #[arg(long)]
        mark_device: Option<String>,
        /// The dmsetup executable.
        #[arg(long, default_value = "/usr/sbin/dmsetup")]
        dmsetup: PathBuf,
        /// Earlier acknowledgements whose sessions to continue.
        #[arg(long)]
        known_acks: Option<PathBuf>,
        /// Sequence number of the first operation.
        #[arg(long, default_value_t = 1)]
        first_seq: u64,
        /// Stop after this many operations.
        #[arg(long)]
        max_ops: Option<u64>,
        /// Stop after this many consecutive failures.
        #[arg(long)]
        stop_after_failures: Option<u32>,
        /// Seed of the operation mix.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Milliseconds the stand-in recognizer takes per chunk.
        #[arg(long, default_value_t = 0)]
        recognizer_delay_ms: u64,
        /// Generations after which a session takes no new work.
        #[arg(long, default_value_t = 40)]
        rotate_after: u64,
        /// Smallest source copy in KiB.
        #[arg(long, default_value_t = 16)]
        source_min_kib: u64,
        /// Largest source copy in KiB.
        #[arg(long, default_value_t = 128)]
        source_max_kib: u64,
        /// Which operations to draw from.
        #[arg(long, value_enum, default_value_t = Mix::All)]
        mix: Mix,
    },
    /// Verifies a session root against acknowledgements.
    Verify {
        /// Session root.
        #[arg(long)]
        root: PathBuf,
        /// Acknowledgements (a file or block device, read up to a NUL byte).
        #[arg(long)]
        acks: PathBuf,
        /// Where the report goes (appended), besides standard output.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Prefix of the report lines.
        #[arg(long, default_value = "VERIFY")]
        prefix: String,
    },
    /// Replays a dm-log-writes log and verifies every flush point.
    Replay {
        /// The write log.
        #[arg(long)]
        log: PathBuf,
        /// The logged device's image as logging began.
        #[arg(long)]
        base: PathBuf,
        /// Scratch directory for the rebuilt images.
        #[arg(long)]
        work: PathBuf,
        /// The workload's acknowledgement log.
        #[arg(long)]
        acks: PathBuf,
        /// Mount point for each copy.
        #[arg(long)]
        mount_point: PathBuf,
        /// Session root inside the filesystem.
        #[arg(long, default_value = "root")]
        root_in_filesystem: PathBuf,
        /// Report file.
        #[arg(long)]
        report: PathBuf,
        /// Check every n-th point.
        #[arg(long, default_value_t = 1)]
        stride: usize,
    },
    /// Assesses one write-error round (layer C).
    Assess {
        /// The round's log, with the harness's `INJECT` line.
        #[arg(long)]
        log: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Arguments::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("vsift-crash-campaign: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(arguments: Arguments) -> Result<bool, CampaignError> {
    match arguments.step {
        Step::Workload {
            root,
            scratch,
            ack_out,
            drain,
            mark_device,
            dmsetup,
            known_acks,
            first_seq,
            max_ops,
            stop_after_failures,
            seed,
            recognizer_delay_ms,
            rotate_after,
            source_min_kib,
            source_max_kib,
            mix,
        } => {
            let config = WorkloadConfig {
                root,
                scratch,
                ack_out,
                drain,
                mark_device,
                dmsetup,
                known_acks,
                first_seq,
                max_ops,
                stop_after_failures,
                seed,
                recognizer_delay: Duration::from_millis(recognizer_delay_ms),
                rotate_after,
                source_kib: (source_min_kib, source_max_kib.max(source_min_kib)),
                mix,
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(CampaignError::io("starting the runtime", "tokio"))?;
            runtime.block_on(workload::run(&config))?;
            Ok(true)
        }
        Step::Verify {
            root,
            acks,
            out,
            prefix,
        } => {
            let events = parse_events(&read_text(&acks, MAX_ACK_BYTES)?);
            if !events.malformed.is_empty() {
                return Err(CampaignError::MalformedAcks(events.malformed));
            }
            let findings = verify::verify(&root, &events.acks, unix_seconds()?);
            let mut text = findings.lines(&prefix).join("\n");
            text.push('\n');
            print!("{text}");
            if let Some(path) = out {
                OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .and_then(|mut file| file.write_all(text.as_bytes()))
                    .map_err(CampaignError::io("writing the verification report", &path))?;
            }
            Ok(findings.clean())
        }
        Step::Replay {
            log,
            base,
            work,
            acks,
            mount_point,
            root_in_filesystem,
            report,
            stride,
        } => {
            let summary = replay::replay(&ReplayConfig {
                log,
                base,
                work,
                acks,
                mount_point,
                root_in_filesystem,
                report,
                stride,
            })?;
            println!("{}", summary.line(0));
            Ok(summary.lost_points == 0
                && summary.damaged_points == 0
                && summary.fsck_failures == 0
                && summary.mount_failures == 0)
        }
        Step::Assess { log } => {
            let assessment = assess::assess(&read_text(&log, MAX_ACK_BYTES)?);
            println!("{}", assessment.line());
            Ok(assessment.passed())
        }
    }
}
