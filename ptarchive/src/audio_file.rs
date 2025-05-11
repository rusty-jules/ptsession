use std::convert::TryFrom;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use ignore::DirEntry;

/// A convenience abstraction for handling audio file paths and names
pub struct AudioFilePath {
    /// The name of the file
    name: String,

    /// The relative path of the file on disk
    path: PathBuf,
}

impl TryFrom<PathBuf> for AudioFilePath {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_from(value: PathBuf) -> Result<Self, Self::Error> {
        Ok(Self {
            name: value
                .file_name()
                .ok_or_else(|| "expected file but given directory")?
                .to_string_lossy()
                .to_string(),
            path: value,
        })
    }
}

impl Deref for AudioFilePath {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl From<DirEntry> for AudioFilePath {
    fn from(value: DirEntry) -> Self {
        Self::try_from(value.into_path()).unwrap()
    }
}

impl AsRef<Path> for AudioFilePath {
    fn as_ref(&self) -> &Path {
        self.path.as_path()
    }
}

impl AudioFilePath {
    #[allow(dead_code)]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn absolute_path_string(&self) -> Result<String, std::io::Error> {
        Ok(self.path.canonicalize()?.display().to_string())
    }

    /// Name without Audio Files prefix
    pub fn file_name(&self) -> &str {
        &self.name
    }

    /// The name of the file as it is intended to be uploaded to the repository.
    /// Notably, oras (and ptarchive) uses the layer image title to construct *relative* path directories on pull.
    /// This is not guaranteed to be the same as the file path
    /// since we may have found the file outside of the Audio Files directory,
    /// but we always upload files as if they had been there for Pro Tools to find
    /// them without requiring a search on its part later.
    pub fn title(&self) -> String {
        format!("Audio Files/{}", self.name)
    }
}
