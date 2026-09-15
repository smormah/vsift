use std::{fmt::Write as _, fs, io, path::Path};

use cap_std::fs::Dir;

pub(crate) struct PrivateStaging {
    directory: Option<Dir>,
    path: std::path::PathBuf,
}

impl PrivateStaging {
    pub(crate) fn new(prefix: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| io::Error::other("test random source unavailable"))?;
        let mut suffix = String::with_capacity(32);
        for byte in random {
            write!(&mut suffix, "{byte:02x}")?;
        }
        let path = std::env::temp_dir().join(format!("{prefix}-{suffix}"));
        fs::create_dir(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }
        let directory = Dir::open_ambient_dir(&path, cap_std::ambient_authority())?;
        Ok(Self {
            directory: Some(directory),
            path,
        })
    }

    pub(crate) fn directory(&self) -> Result<&Dir, io::Error> {
        self.directory
            .as_ref()
            .ok_or_else(|| io::Error::other("test staging directory is closed"))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrivateStaging {
    fn drop(&mut self) {
        self.directory.take();
        let _ = fs::remove_dir_all(&self.path);
    }
}
