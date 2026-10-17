//! A temporary folder for the crate's unit tests that removes itself (#423).
//!
//! A test that cleans up only at its end leaves its folder behind when an
//! assertion fails first. This guard is removed when it drops, also while a
//! failed test unwinds. It removes only the folder it created: the name ends
//! in 32 random hexadecimal digits and `create_dir` fails on a folder that
//! already exists, so it never adopts, and never deletes, anything else.
//!
//! On Windows a folder that something still holds open cannot be removed, so
//! a guard must drop after every handle into its folder: declare it before
//! them, or as the last field of a struct.

use std::{
    error::Error,
    fmt::Write as _,
    fs,
    ops::Deref,
    path::{Path, PathBuf},
};

/// A folder under the temporary folder, made by [`TestFolder::new`] and
/// removed with its contents when the guard drops.
pub(crate) struct TestFolder(PathBuf);

impl TestFolder {
    /// Creates `<temporary folder>/<prefix>-<32 random hexadecimal digits>`.
    pub(crate) fn new(prefix: &str) -> Result<Self, Box<dyn Error>> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| std::io::Error::other("random source failed"))?;
        let mut name = format!("{prefix}-");
        for byte in random {
            write!(&mut name, "{byte:02x}")?;
        }
        let path = std::env::temp_dir().join(name);
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Deref for TestFolder {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TestFolder {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestFolder {
    fn drop(&mut self) {
        // A test that removed the folder itself leaves nothing to remove.
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs};

    use super::TestFolder;

    #[test]
    fn the_folder_and_its_contents_go_when_the_guard_drops() -> Result<(), Box<dyn Error>> {
        let folder = TestFolder::new("vsift-test-folder")?;
        let path = folder.to_path_buf();
        fs::create_dir(path.join("inner"))?;
        fs::write(path.join("inner").join("file"), b"contents")?;
        drop(folder);
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn the_folder_goes_while_a_failed_test_unwinds() -> Result<(), Box<dyn Error>> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let unwound = std::thread::spawn(move || -> Result<(), String> {
            let folder = TestFolder::new("vsift-test-folder").map_err(|error| error.to_string())?;
            sender
                .send(folder.to_path_buf())
                .map_err(|error| error.to_string())?;
            fs::write(folder.join("file"), b"contents").map_err(|error| error.to_string())?;
            // A failing assertion of a test: the thread unwinds past the guard.
            assert!(folder.as_os_str().is_empty(), "a failing assertion");
            Ok(())
        })
        .join();
        assert!(unwound.is_err(), "the thread did not fail");
        let path = receiver.recv()?;
        assert!(!path.exists());
        Ok(())
    }
}
