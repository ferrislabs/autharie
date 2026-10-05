use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use autharie_domain::CoreError;
use uuid::Uuid;

pub struct Workdir {
    path: PathBuf,
}

impl Workdir {
    pub fn create() -> Result<Self, CoreError> {
        let path = std::env::temp_dir().join(format!("autharie-bootstrap-{}", Uuid::new_v4()));
        DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|e| failed("create a private directory", e))?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write_private(&self, name: &str, content: &[u8]) -> Result<PathBuf, CoreError> {
        let file = self.path.join(name);
        let mut handle = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
            .map_err(|e| failed("create a private file", e))?;
        handle
            .write_all(content)
            .map_err(|e| failed("write a private file", e))?;
        Ok(file)
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn failed(what: &str, detail: std::io::Error) -> CoreError {
    CoreError::InternalError(format!(
        "could not {what} for the bootstrap: {}",
        detail.kind()
    ))
}
