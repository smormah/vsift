//! A stand-in for `vsift-ocr-runner` in the OCR adapter's tests (`tests/ocr_runner.rs`).
//! It is never shipped or run outside those tests.
//!
//! `VSift` runs the runner with a closed argument list and an empty environment,
//! so the stand-in reads its behaviour from its own file name,
//! `ocr-<behaviour>`, as the smoke fixture does. It uses only the standard
//! library.
//!
//! Behaviours:
//! - `good`: `describe` answers protocol 1 with fixed digests; `read` reads each
//!   image's size from its PNG header and writes one line per image whose text
//!   is the file's bytes after the header, as UTF-8 (the tests choose them);
//! - `garbage`: writes text that is not the protocol;
//! - `hang`: sleeps past any deadline;
//! - `nomodel`: exits 4, the runner's "model unavailable";
//! - `crash`: aborts, as a crashed or killed runner ends;
//! - `liar`: answers for one image fewer than it was given.

use std::{
    env, fs,
    io::{self, Write},
    process::ExitCode,
    thread,
    time::Duration,
};

const HELLO: &str = r#"{"record":"runner","protocol":1,"runner_version":"0.1.0"}"#;
/// Bytes of a PNG signature and its `IHDR` chunk up to the end of the height.
const HEADER: usize = 24;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(_) => ExitCode::from(70),
    }
}

fn run() -> io::Result<ExitCode> {
    let executable = env::current_exe()?;
    let stem = executable
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_owned();
    let behaviour = stem.strip_prefix("ocr-").unwrap_or("good").to_owned();
    let arguments: Vec<String> = env::args().skip(1).collect();
    let mut stdout = io::stdout().lock();
    match behaviour.as_str() {
        "garbage" => {
            writeln!(stdout, "this is not the runner protocol")?;
            return Ok(ExitCode::SUCCESS);
        }
        "hang" => {
            thread::sleep(Duration::from_secs(600));
            return Ok(ExitCode::SUCCESS);
        }
        "nomodel" => return Ok(ExitCode::from(4)),
        "crash" => std::process::abort(),
        _ => {}
    }
    match arguments.first().map(String::as_str) {
        Some("describe") => {
            writeln!(
                stdout,
                r#"{{"record":"describe","protocol":1,"runner_version":"0.1.0","runtime_version":"1.31.0","profile":"ppocrv5-latin-v1","detector_sha256":"{}","recognizer_sha256":"{}"}}"#,
                "d".repeat(64),
                "e".repeat(64)
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Some("read") => {
            let images: Vec<&String> = arguments.iter().skip(5).collect();
            let answered = if behaviour == "liar" {
                images.len().saturating_sub(1)
            } else {
                images.len()
            };
            writeln!(stdout, "{HELLO}")?;
            for (index, path) in images.iter().take(answered).enumerate() {
                let bytes = fs::read(path)?;
                let (width, height) = size(&bytes);
                let text = String::from_utf8_lossy(bytes.get(HEADER..).unwrap_or_default())
                    .replace(['"', '\\'], "");
                writeln!(
                    stdout,
                    r#"{{"record":"line","image":{index},"x":1,"y":1,"width":10,"height":5,"text":"{text}","text_cut":false,"confidence":0.9}}"#
                )?;
                writeln!(
                    stdout,
                    r#"{{"record":"image","image":{index},"width":{width},"height":{height},"lines":1,"truncated":false}}"#
                )?;
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => Ok(ExitCode::from(2)),
    }
}

fn size(bytes: &[u8]) -> (u32, u32) {
    let field = |at: usize| {
        bytes
            .get(at..at + 4)
            .and_then(|field| <[u8; 4]>::try_from(field).ok())
            .map_or(0, u32::from_be_bytes)
    };
    (field(16), field(20))
}
