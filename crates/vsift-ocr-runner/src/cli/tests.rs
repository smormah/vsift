use std::{
    cell::Cell,
    env,
    ffi::OsString,
    fs,
    num::NonZeroU16,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{Backend, Command, Layout, MAX_IMAGES, run};
use crate::{
    RunnerFailure,
    pipeline::{Inference, InferenceError, Model},
    png::tests::encode,
    recognition::Dictionary,
    tensor::Tensor,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const OWNED_PREFIX: &str = "vsift-ocr-runner-cli-test-";
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A folder of the test's own, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new() -> Built<Self> {
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Folder {
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

fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// A detector that finds no text, or fails, on every frame.
struct Blank {
    fail: bool,
}

impl Inference for Blank {
    fn run(&mut self, model: Model, input: &Tensor) -> Result<Tensor, InferenceError> {
        if self.fail {
            return Err(InferenceError { model });
        }
        let shape = input.shape();
        let (height, width) = (
            shape.get(2).copied().unwrap_or(0),
            shape.get(3).copied().unwrap_or(0),
        );
        Tensor::new(vec![1, 1, height, width], vec![0.0; height * width])
            .map_err(|_| InferenceError { model })
    }
}

/// A backend whose runtime is version `1.31.0`, or missing, and whose session is [`Blank`].
struct Scripted {
    runtime: bool,
    fail: bool,
    opened: Cell<u16>,
}

impl Scripted {
    const fn new(runtime: bool, fail: bool) -> Self {
        Self {
            runtime,
            fail,
            opened: Cell::new(0),
        }
    }
}

impl Backend for Scripted {
    type Session = Blank;

    fn runtime_version(&self, _layout: &Layout) -> Result<String, RunnerFailure> {
        if self.runtime {
            Ok(String::from("1.31.0"))
        } else {
            Err(RunnerFailure::RuntimeUnavailable)
        }
    }

    fn open(
        &self,
        layout: &Layout,
        threads: NonZeroU16,
    ) -> Result<(Blank, Dictionary), RunnerFailure> {
        self.runtime_version(layout)?;
        self.opened.set(threads.get());
        let dictionary =
            Dictionary::from_lines("a\nb").map_err(|_| RunnerFailure::ModelUnavailable)?;
        Ok((Blank { fail: self.fail }, dictionary))
    }
}

fn installed(folder: &Folder) -> Built<Layout> {
    let layout = Layout::beside(&folder.0);
    if let Some(models) = layout.detector.parent() {
        fs::create_dir_all(models)?;
    }
    fs::write(&layout.detector, b"detector bytes")?;
    fs::write(&layout.recognizer, b"recognizer bytes")?;
    Ok(layout)
}

fn lines(stdout: &[u8]) -> Built<Vec<serde_json::Value>> {
    Ok(std::str::from_utf8(stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?)
}

#[test]
fn only_the_two_command_forms_within_their_bounds_are_accepted() {
    assert_eq!(
        Command::parse(&arguments(&["describe"])),
        Ok(Command::Describe)
    );
    let mut many = vec!["read", "--threads", "2", "--max-lines", "256"];
    many.extend(std::iter::repeat_n("a.png", MAX_IMAGES));
    assert!(Command::parse(&arguments(&many)).is_ok());
    many.push("b.png");
    for refused in [
        many,
        vec![],
        vec!["describe", "extra"],
        vec!["read", "--threads", "2", "--max-lines", "256"],
        vec!["read", "--threads", "0", "--max-lines", "256", "a.png"],
        vec!["read", "--threads", "9", "--max-lines", "256", "a.png"],
        vec!["read", "--threads", "2", "--max-lines", "0", "a.png"],
        vec!["read", "--threads", "2", "--max-lines", "257", "a.png"],
        vec!["read", "--max-lines", "2", "--threads", "2", "a.png"],
        vec!["help"],
    ] {
        assert_eq!(
            Command::parse(&arguments(&refused)),
            Err(RunnerFailure::Usage),
            "{refused:?}"
        );
    }
}

#[test]
fn describe_names_the_build_by_its_versions_and_model_digests() -> TestResult {
    let folder = Folder::new()?;
    let layout = installed(&folder)?;
    let mut stdout = Vec::new();
    let status = run(
        &arguments(&["describe"]),
        &layout,
        &Scripted::new(true, false),
        &mut stdout,
    );
    assert_eq!(status, 0);
    let records = lines(&stdout)?;
    assert_eq!(records.len(), 1);
    let record = records.first().ok_or("no record")?;
    assert_eq!(record["record"], "describe");
    assert_eq!(record["protocol"], 1);
    assert_eq!(record["runtime_version"], "1.31.0");
    assert_eq!(record["profile"], "ppocrv5-latin-v1");
    // SHA-256 of the bytes "detector bytes".
    assert_eq!(
        record["detector_sha256"],
        "1157d519da9b32785f9089e5f3246268f92403102d5fa18f33c3fd640889e62d"
    );
    assert_ne!(record["detector_sha256"], record["recognizer_sha256"]);
    Ok(())
}

#[test]
fn a_missing_model_or_runtime_is_one_error_record_and_its_status() -> TestResult {
    let folder = Folder::new()?;
    let layout = Layout::beside(&folder.0);
    let mut stdout = Vec::new();
    let status = run(
        &arguments(&["describe"]),
        &layout,
        &Scripted::new(true, false),
        &mut stdout,
    );
    assert_eq!(status, RunnerFailure::ModelUnavailable.exit_status());
    assert_eq!(
        String::from_utf8(stdout)?,
        "{\"record\":\"error\",\"image\":null,\"code\":\"model_unavailable\"}\n"
    );
    let layout = installed(&folder)?;
    let mut stdout = Vec::new();
    let status = run(
        &arguments(&["read", "--threads", "2", "--max-lines", "8", "a.png"]),
        &layout,
        &Scripted::new(false, false),
        &mut stdout,
    );
    assert_eq!(status, RunnerFailure::RuntimeUnavailable.exit_status());
    let records = lines(&stdout)?;
    assert_eq!(
        records.first().map(|record| &record["record"]),
        Some(&"runner".into())
    );
    assert_eq!(
        records.get(1).map(|record| &record["code"]),
        Some(&"runtime_unavailable".into())
    );
    Ok(())
}

#[test]
fn read_answers_every_image_in_order_and_goes_on_after_a_bad_one() -> TestResult {
    let folder = Folder::new()?;
    let layout = installed(&folder)?;
    let good = folder.0.join("good.png");
    fs::write(
        &good,
        encode(64, 32, &vec![255; 64 * 32 * 3], 0, [8, 2, 0, 0, 0]),
    )?;
    let bad = folder.0.join("bad.png");
    fs::write(&bad, b"not a png")?;
    let missing = folder.0.join("missing.png");
    let paths = [&good, &bad, &missing, &good];
    let mut values = vec![
        OsString::from("read"),
        OsString::from("--threads"),
        OsString::from("3"),
        OsString::from("--max-lines"),
        OsString::from("8"),
    ];
    values.extend(paths.iter().map(|path| path.as_os_str().to_owned()));
    let backend = Scripted::new(true, false);
    let mut stdout = Vec::new();
    assert_eq!(run(&values, &layout, &backend, &mut stdout), 0);
    assert_eq!(backend.opened.get(), 3);
    let records = lines(&stdout)?;
    let kinds: Vec<(&str, i64)> = records
        .iter()
        .skip(1)
        .map(|record| {
            (
                record["record"].as_str().unwrap_or_default(),
                record["image"].as_i64().unwrap_or(-1),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        vec![("image", 0), ("error", 1), ("error", 2), ("image", 3)]
    );
    assert_eq!(
        records.get(1).map(|record| &record["width"]),
        Some(&64.into())
    );
    assert_eq!(
        records.get(2).map(|record| &record["code"]),
        Some(&"image_invalid".into())
    );

    let failing = Scripted::new(true, true);
    let mut stdout = Vec::new();
    assert_eq!(run(&values, &layout, &failing, &mut stdout), 0);
    let records = lines(&stdout)?;
    assert_eq!(
        records.get(1).map(|record| &record["code"]),
        Some(&"inference_failed".into())
    );
    Ok(())
}
