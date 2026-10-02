//! Helpers shared by the harness's integration tests.
//!
//! Scratch folders are named with the process id, an atomic counter and the
//! clock, and created with `create_dir`, so two tests that start in the same
//! microsecond (macOS) never share one (issue #205).

#![allow(dead_code, reason = "Each test file uses some of these helpers")]

use std::{
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Distinguishes the scratch folders one test process creates.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// The repository root.
pub fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// A scratch folder below the system temporary directory, removed on drop.
pub struct Scratch {
    root: PathBuf,
}

impl Scratch {
    /// Makes a new, empty folder.
    pub fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scratch-{label}-{}-{sequence}-{stamp}",
            std::process::id()
        ));
        // `create_dir`, not `create_dir_all`: a folder that exists belongs to
        // someone else and must fail loudly, never be shared.
        fs::create_dir(&root).map_err(|error| format!("{}: {error}", root.display()))?;
        Ok(Self { root })
    }

    /// The folder.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// The folder above it: where a cold scan stops looking.
    pub fn parent(&self) -> Option<PathBuf> {
        self.root.parent().map(Path::to_path_buf)
    }

    /// Writes a file below the folder, creating its folders.
    pub fn write(&self, relative: &str, text: &str) -> io::Result<PathBuf> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, text)?;
        Ok(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // VSIFT_TEST_KEEP=1 keeps the folder for a look after a failure.
        if std::env::var_os("VSIFT_TEST_KEEP").is_some() {
            eprintln!("kept {}", self.root.display());
            return;
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
