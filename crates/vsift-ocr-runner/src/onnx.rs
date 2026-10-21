//! The ONNX Runtime backend: the two models run through `ort`'s safe API
//! (ADR 0030 decision 1).
//!
//! The runtime library is loaded at run time from the runner's own
//! `onnxruntime` folder ([`Layout::runtime`]), never linked at build time and
//! never looked for anywhere else, so the runner uses exactly the library it
//! was installed with. Nothing here needs `unsafe`: `ort` holds the C API's
//! raw calls, and this crate forbids `unsafe` code.
//!
//! The runner's own safety net is around what `ort` can refuse: the library
//! is loaded before any other `ort` call (a later call would otherwise panic on
//! a missing library), a model file is bounded before it is parsed, and every
//! runtime failure becomes a typed [`RunnerFailure`] or [`InferenceError`].

use std::{fs, num::NonZeroU16, path::Path};

use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::Tensor as OrtTensor,
};

use crate::{
    RunnerFailure,
    cli::{Backend, Layout, MAX_MODEL_BYTES},
    pipeline::{Inference, InferenceError, Model},
    recognition::Dictionary,
    tensor::Tensor,
};

/// The ONNX Runtime library's file name in [`Layout::runtime`], as Microsoft's
/// 1.31.0 archive names it in its `lib` folder.
#[cfg(target_os = "windows")]
pub const RUNTIME_LIBRARY: &str = "onnxruntime.dll";
/// See the Windows definition.
#[cfg(target_os = "macos")]
pub const RUNTIME_LIBRARY: &str = "libonnxruntime.1.31.0.dylib";
/// See the Windows definition.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub const RUNTIME_LIBRARY: &str = "libonnxruntime.so.1.31.0";

/// The recognizer model's metadata key that lists its characters, one a line.
pub const DICTIONARY_KEY: &str = "character";

/// ONNX Runtime, loaded from the runner's folder.
#[derive(Clone, Copy, Debug, Default)]
pub struct OnnxRuntime;

/// The two loaded models.
#[derive(Debug)]
pub struct OnnxSession {
    detector: Session,
    recognizer: Session,
}

impl Backend for OnnxRuntime {
    type Session = OnnxSession;

    fn runtime_version(&self, layout: &Layout) -> Result<String, RunnerFailure> {
        load(layout)?;
        let info = ort::info();
        version_from_build_info(info).ok_or_else(|| {
            diagnose(&format!(
                "the runtime's build information names no release: {}",
                info.chars().take(240).collect::<String>()
            ));
            RunnerFailure::RuntimeUnavailable
        })
    }

    fn open(
        &self,
        layout: &Layout,
        threads: NonZeroU16,
    ) -> Result<(OnnxSession, Dictionary), RunnerFailure> {
        load(layout)?;
        let detector = session(&layout.detector, threads)?;
        let recognizer = session(&layout.recognizer, threads)?;
        let listed = recognizer
            .metadata()
            .ok()
            .and_then(|metadata| metadata.custom(DICTIONARY_KEY))
            .ok_or(RunnerFailure::ModelUnavailable)?;
        let dictionary =
            Dictionary::from_lines(&listed).map_err(|_| RunnerFailure::ModelUnavailable)?;
        Ok((
            OnnxSession {
                detector,
                recognizer,
            },
            dictionary,
        ))
    }
}

/// One line on standard error for whoever runs the runner by hand or reads a
/// hosted log; `VSift` keeps standard error bounded and never shows it as data.
fn diagnose(message: &str) {
    eprintln!("vsift-ocr-runner: {message}");
}

/// Loads the library once; later calls find it loaded.
fn load(layout: &Layout) -> Result<(), RunnerFailure> {
    let library = layout.runtime.join(RUNTIME_LIBRARY);
    if !library.is_file() {
        return Err(RunnerFailure::RuntimeUnavailable);
    }
    let _applied = ort::init_from(&library)
        .map_err(|error| {
            diagnose(&format!("ONNX Runtime could not be loaded: {error}"));
            RunnerFailure::RuntimeUnavailable
        })?
        .with_name("vsift-ocr-runner")
        .commit();
    Ok(())
}

/// A session of `path`, at most [`MAX_MODEL_BYTES`], with `threads` threads
/// inside each operator and none across them, as the evaluation ran it.
fn session(path: &Path, threads: NonZeroU16) -> Result<Session, RunnerFailure> {
    let size = fs::metadata(path)
        .map_err(|_| RunnerFailure::ModelUnavailable)?
        .len();
    if size == 0 || size > MAX_MODEL_BYTES {
        return Err(RunnerFailure::ModelUnavailable);
    }
    Session::builder()
        .and_then(|builder| {
            builder
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(Into::into)
        })
        .and_then(|builder| {
            builder
                .with_intra_threads(usize::from(threads.get()))
                .map_err(Into::into)
        })
        .and_then(|builder| builder.with_inter_threads(1).map_err(Into::into))
        .and_then(|mut builder| builder.commit_from_file(path))
        .map_err(|error| {
            diagnose(&format!("a model could not be loaded: {error}"));
            RunnerFailure::ModelUnavailable
        })
}

impl Inference for OnnxSession {
    fn run(&mut self, model: Model, input: &Tensor) -> Result<Tensor, InferenceError> {
        let failed = || InferenceError { model };
        let session = match model {
            Model::Detector => &mut self.detector,
            Model::Recognizer => &mut self.recognizer,
        };
        let value = OrtTensor::from_array((input.shape().to_vec(), input.data().to_vec()))
            .map_err(|_| failed())?;
        let outputs = session.run(ort::inputs![value]).map_err(|_| failed())?;
        let output = outputs.values().next().ok_or_else(failed)?;
        let (shape, data) = output.try_extract_tensor::<f32>().map_err(|_| failed())?;
        let dimensions = shape
            .iter()
            .map(|dimension| usize::try_from(*dimension))
            .collect::<Result<Vec<usize>, _>>()
            .map_err(|_| failed())?;
        Tensor::new(dimensions, data.to_vec()).map_err(|_| failed())
    }
}

/// The runtime's version from its build information, whose release branch is
/// `rel-<major>.<minor>.<patch>` (for example `git-branch=rel-1.31.0`); `None`
/// when it does not say, so a build that cannot name its runtime is refused.
#[must_use]
pub fn version_from_build_info(info: &str) -> Option<String> {
    let start = info.find("git-branch=rel-")? + "git-branch=rel-".len();
    let version: String = info
        .get(start..)?
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let parts: Vec<&str> = version.split('.').collect();
    (parts.len() == 3 && parts.iter().all(|part| !part.is_empty()) && version.len() <= 32)
        .then_some(version)
}

#[cfg(test)]
mod tests {
    use super::version_from_build_info;

    #[test]
    fn the_runtime_version_is_its_release_branch() {
        assert_eq!(
            version_from_build_info(
                "ORT Build Info: git-branch=rel-1.31.0, git-commit-id=abc, build type=Release"
            ),
            Some(String::from("1.31.0"))
        );
        for unnamed in [
            "",
            "ORT Build Info: git-branch=main, git-commit-id=abc",
            "git-branch=rel-1.31",
            "git-branch=rel-1..0",
        ] {
            assert_eq!(version_from_build_info(unnamed), None, "{unnamed}");
        }
    }
}
