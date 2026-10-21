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
    cli::{Backend, Layout, MAX_MODEL_BYTES, file_sha256},
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
        let library = layout.runtime.join(RUNTIME_LIBRARY);
        let digest = file_sha256(&library, MAX_RUNTIME_LIBRARY_BYTES).map_err(|_| {
            diagnose("the runtime library could not be read to name it");
            RunnerFailure::RuntimeUnavailable
        })?;
        Ok(runtime_version_of(&digest))
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

/// The ONNX Runtime libraries the runner is released with, by the SHA-256 of
/// the library file and the version Microsoft's archive names. The runtime's
/// own build information names no release (`git-branch=HEAD` in 1.31.0), and its
/// version string is reached only through the C API's raw calls, so the runner
/// names the library by its bytes: a pinned library is its version; any other is
/// `sha256-` and the first 16 hexadecimal digits of its digest, so a different
/// library is always a different engine build and never claims a version.
pub const KNOWN_RUNTIMES: &[(&str, &str)] = &[
    // onnxruntime-linux-x64-1.31.0.tgz, lib/libonnxruntime.so.1.31.0 (hosted
    // staging of `P16 OCR runner`, run 39597504891).
    (
        "469849977cb60744ea160927e1a3a0643fd4c55c0aff743251c6d293402e334e",
        "1.31.0",
    ),
    // onnxruntime-win-x64-1.31.0.zip, lib/onnxruntime.dll (run 39598128889).
    (
        "0c42dc4e278f0968fdf9ebf1b1e54a74c657a9942e70803e453c91993451cf4c",
        "1.31.0",
    ),
];

/// Largest runtime library the runner hashes.
pub const MAX_RUNTIME_LIBRARY_BYTES: u64 = 256 * 1024 * 1024;

/// The version label of a runtime library of SHA-256 `digest` (see [`KNOWN_RUNTIMES`]).
#[must_use]
pub fn runtime_version_of(digest: &str) -> String {
    KNOWN_RUNTIMES
        .iter()
        .find(|(known, _)| *known == digest)
        .map_or_else(
            || format!("sha256-{}", digest.chars().take(16).collect::<String>()),
            |(_, version)| (*version).to_owned(),
        )
}

#[cfg(test)]
mod tests {
    use super::runtime_version_of;

    #[test]
    fn a_pinned_library_is_its_version_and_any_other_is_named_by_its_digest() {
        assert_eq!(
            runtime_version_of("469849977cb60744ea160927e1a3a0643fd4c55c0aff743251c6d293402e334e"),
            "1.31.0"
        );
        assert_eq!(
            runtime_version_of("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            "sha256-0123456789abcdef"
        );
    }
}
