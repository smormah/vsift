use std::{
    cell::Cell,
    env,
    ffi::OsString,
    fs,
    num::NonZeroU16,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{
    Backend, Command, DETECTOR_FILE, Layout, MAX_IMAGES, MODELS_FOLDER, RECOGNIZER_FILE,
    RUNTIME_FOLDER, RUNTIME_LIBRARY, run,
};
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

    fn runtime_version(
        &self,
        _layout: &Layout,
        _library_sha256: &str,
    ) -> Result<String, RunnerFailure> {
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
        self.runtime_version(layout, "")?;
        self.opened.set(threads.get());
        let dictionary =
            Dictionary::from_lines("a\nb").map_err(|_| RunnerFailure::ModelUnavailable)?;
        Ok((Blank { fail: self.fail }, dictionary))
    }
}

/// SHA-256 of the bytes `runtime bytes`, `detector bytes` and `recognizer bytes`.
const RUNTIME_SHA256: &str = "7fc5036745ec73b6169495830cf3ef3b58127addf27fcc0d0b8fe87b4de5808c";
const DETECTOR_SHA256: &str = "1157d519da9b32785f9089e5f3246268f92403102d5fa18f33c3fd640889e62d";
const RECOGNIZER_SHA256: &str = "3fb1c140febdef7006834ed9d4e7b93c9a0efadeb241a45f2d388838d7792edf";

/// A runner installed as a release archive unpacks: `onnxruntime/` and `models/`.
fn installed(folder: &Folder) -> Built<Layout> {
    for sub in [RUNTIME_FOLDER, MODELS_FOLDER] {
        fs::create_dir_all(folder.0.join(sub))?;
    }
    let layout = Layout::beside(&folder.0);
    fs::write(&layout.runtime_library, b"runtime bytes")?;
    fs::write(&layout.detector, b"detector bytes")?;
    fs::write(&layout.recognizer, b"recognizer bytes")?;
    Ok(Layout::beside(&folder.0))
}

/// A runner installed as the managed store keeps a component: every file beside it.
fn installed_flat(folder: &Folder) -> Built<Layout> {
    fs::write(folder.0.join(RUNTIME_LIBRARY), b"runtime bytes")?;
    fs::write(folder.0.join(DETECTOR_FILE), b"detector bytes")?;
    fs::write(folder.0.join(RECOGNIZER_FILE), b"recognizer bytes")?;
    Ok(Layout::beside(&folder.0))
}

/// `read`'s arguments for `images`, two threads, eight lines.
fn read_arguments(images: &[&PathBuf]) -> Vec<OsString> {
    let mut values = arguments(&["read", "--threads", "2", "--max-lines", "8"]);
    values.extend(images.iter().map(|path| path.as_os_str().to_owned()));
    values
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
    assert_eq!(record["runtime_sha256"], RUNTIME_SHA256);
    assert_eq!(record["detector_sha256"], DETECTOR_SHA256);
    assert_eq!(record["recognizer_sha256"], RECOGNIZER_SHA256);
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
    // A runtime that cannot be loaded ends `read` before its greeting, which
    // names only a build whose files were hashed and whose runtime loaded.
    let layout = installed(&folder)?;
    let mut stdout = Vec::new();
    let status = run(
        &arguments(&["read", "--threads", "2", "--max-lines", "8", "a.png"]),
        &layout,
        &Scripted::new(false, false),
        &mut stdout,
    );
    assert_eq!(status, RunnerFailure::RuntimeUnavailable.exit_status());
    assert_eq!(
        String::from_utf8(stdout)?,
        "{\"record\":\"error\",\"image\":null,\"code\":\"runtime_unavailable\"}\n"
    );
    // So does a library that is not there to be hashed.
    fs::remove_file(&layout.runtime_library)?;
    let mut stdout = Vec::new();
    let status = run(
        &arguments(&["read", "--threads", "2", "--max-lines", "8", "a.png"]),
        &layout,
        &Scripted::new(true, false),
        &mut stdout,
    );
    assert_eq!(status, RunnerFailure::RuntimeUnavailable.exit_status());
    Ok(())
}

/// RV-2 finding 3: `read` hashes the runtime and both models itself before it
/// loads them, and its greeting names that build, so `VSift` can refuse a run by
/// another build than `describe` reported. A model changed between the two
/// runs is named by its new digest.
#[test]
fn reads_greeting_names_the_build_it_hashed_before_loading() -> TestResult {
    let folder = Folder::new()?;
    let layout = installed(&folder)?;
    let image = folder.0.join("frame.png");
    fs::write(
        &image,
        encode(64, 32, &vec![255; 64 * 32 * 3], 0, [8, 2, 0, 0, 0]),
    )?;
    let mut stdout = Vec::new();
    assert_eq!(
        run(
            &read_arguments(&[&image]),
            &layout,
            &Scripted::new(true, false),
            &mut stdout
        ),
        0
    );
    let records = lines(&stdout)?;
    let greeting = records.first().ok_or("no greeting")?;
    assert_eq!(
        greeting,
        &serde_json::json!({
            "record": "runner",
            "protocol": 1,
            "runner_version": super::RUNNER_VERSION,
            "runtime_version": "1.31.0",
            "runtime_sha256": RUNTIME_SHA256,
            "detector_sha256": DETECTOR_SHA256,
            "recognizer_sha256": RECOGNIZER_SHA256,
        })
    );
    fs::write(&layout.detector, b"changed detector")?;
    let mut stdout = Vec::new();
    assert_eq!(
        run(
            &read_arguments(&[&image]),
            &layout,
            &Scripted::new(true, false),
            &mut stdout
        ),
        0
    );
    let records = lines(&stdout)?;
    assert_eq!(
        records.first().map(|record| &record["detector_sha256"]),
        Some(&"35803e6bc5a68d8b0b3706637677a562ea57247de83acc857510256c030879e8".into())
    );
    Ok(())
}

/// The managed store's flat folder (the planner's decision of 2026-10-22):
/// `describe` and `read` find the runtime and models beside the executable and
/// name the same build from them as from a release archive's subfolders.
#[test]
fn the_flat_layout_names_the_same_build_as_the_nested_one() -> TestResult {
    let image_folder = Folder::new()?;
    let image = image_folder.0.join("frame.png");
    fs::write(
        &image,
        encode(64, 32, &vec![255; 64 * 32 * 3], 0, [8, 2, 0, 0, 0]),
    )?;
    let mut answers = Vec::new();
    for flat in [false, true] {
        let folder = Folder::new()?;
        let layout = if flat {
            installed_flat(&folder)?
        } else {
            installed(&folder)?
        };
        assert_eq!(
            layout.detector.parent() == Some(folder.0.as_path()),
            flat,
            "{layout:?}"
        );
        let mut described = Vec::new();
        let backend = Scripted::new(true, false);
        assert_eq!(
            run(&arguments(&["describe"]), &layout, &backend, &mut described),
            0
        );
        let mut read = Vec::new();
        assert_eq!(
            run(&read_arguments(&[&image]), &layout, &backend, &mut read),
            0
        );
        let description = lines(&described)?.into_iter().next().ok_or("no record")?;
        let greeting = lines(&read)?.into_iter().next().ok_or("no greeting")?;
        for key in ["runtime_sha256", "detector_sha256", "recognizer_sha256"] {
            assert_eq!(description[key], greeting[key], "{key}, flat: {flat}");
        }
        assert_eq!(greeting["runtime_sha256"], RUNTIME_SHA256);
        answers.push(greeting);
    }
    assert_eq!(answers.first(), answers.get(1));
    Ok(())
}

/// RV-2 finding 1, end to end: a frame whose detection map would break its
/// bound is refused as the image's fault, and the run goes on.
#[test]
fn a_frame_whose_map_cannot_be_bounded_is_an_invalid_image() -> TestResult {
    let folder = Folder::new()?;
    let layout = installed(&folder)?;
    // 1 x 4000: reduced to 2000 tall, its 1-pixel side becomes 0 and cannot be
    // enlarged (before RV-2 this was reported as the models' failure).
    let narrow = folder.0.join("narrow.png");
    fs::write(
        &narrow,
        encode(1, 4000, &vec![255; 4000 * 3], 0, [8, 2, 0, 0, 0]),
    )?;
    let good = folder.0.join("good.png");
    fs::write(
        &good,
        encode(64, 32, &vec![255; 64 * 32 * 3], 0, [8, 2, 0, 0, 0]),
    )?;
    let mut stdout = Vec::new();
    assert_eq!(
        run(
            &read_arguments(&[&narrow, &good]),
            &layout,
            &Scripted::new(true, false),
            &mut stdout
        ),
        0
    );
    let records = lines(&stdout)?;
    assert_eq!(
        records
            .get(1)
            .map(|record| (&record["record"], &record["code"])),
        Some((&"error".into(), &"image_invalid".into()))
    );
    assert_eq!(
        records.get(2).map(|record| &record["record"]),
        Some(&"image".into())
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

#[test]
fn files_are_found_in_their_subfolders_first_then_beside_the_executable() -> TestResult {
    let folder = Folder::new()?;
    let flat = |name: &str| folder.0.join(name);
    let nested = |sub: &str, name: &str| folder.0.join(sub).join(name);
    // Nothing installed: the subfolder paths, so a failure names them.
    let empty = Layout::beside(&folder.0);
    assert_eq!(
        empty.runtime_library,
        nested(RUNTIME_FOLDER, RUNTIME_LIBRARY)
    );
    assert_eq!(empty.detector, nested(MODELS_FOLDER, DETECTOR_FILE));
    // The managed store's flat folder: every file beside the executable.
    for name in [RUNTIME_LIBRARY, DETECTOR_FILE, RECOGNIZER_FILE] {
        fs::write(flat(name), b"flat")?;
    }
    let managed = Layout::beside(&folder.0);
    assert_eq!(managed.runtime_library, flat(RUNTIME_LIBRARY));
    assert_eq!(managed.detector, flat(DETECTOR_FILE));
    assert_eq!(managed.recognizer, flat(RECOGNIZER_FILE));
    // A release archive's subfolders win over files beside the executable.
    fs::create_dir_all(nested(RUNTIME_FOLDER, ""))?;
    fs::create_dir_all(nested(MODELS_FOLDER, ""))?;
    for (sub, name) in [
        (RUNTIME_FOLDER, RUNTIME_LIBRARY),
        (MODELS_FOLDER, DETECTOR_FILE),
        (MODELS_FOLDER, RECOGNIZER_FILE),
    ] {
        fs::write(nested(sub, name), b"nested")?;
    }
    let archive = Layout::beside(&folder.0);
    assert_eq!(
        archive.runtime_library,
        nested(RUNTIME_FOLDER, RUNTIME_LIBRARY)
    );
    assert_eq!(archive.recognizer, nested(MODELS_FOLDER, RECOGNIZER_FILE));
    Ok(())
}

#[test]
fn describe_works_from_the_managed_stores_flat_folder() -> TestResult {
    let folder = Folder::new()?;
    installed_flat(&folder)?;
    let mut stdout = Vec::new();
    let status = run(
        &arguments(&["describe"]),
        &Layout::beside(&folder.0),
        &Scripted::new(true, false),
        &mut stdout,
    );
    assert_eq!(status, 0);
    let records = lines(&stdout)?;
    assert_eq!(
        records.first().map(|record| &record["detector_sha256"]),
        Some(&"1157d519da9b32785f9089e5f3246268f92403102d5fa18f33c3fd640889e62d".into())
    );
    Ok(())
}
