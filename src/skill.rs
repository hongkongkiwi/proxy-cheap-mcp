//! Bundled agent skill.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

pub const SKILL_MD: &str = include_str!("../skill/SKILL.md");

pub fn default_skill_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agents")
        .join("skills")
        .join("proxy-cheap")
}

/// Write the bundled skill to `directory/SKILL.md`.
pub fn install(directory: Option<&Path>) -> Result<PathBuf> {
    let directory = directory
        .map(Path::to_path_buf)
        .unwrap_or_else(default_skill_dir);
    fs::create_dir_all(&directory).map_err(|error| {
        Error::Config(format!("could not create {}: {error}", directory.display()))
    })?;
    let path = directory.join("SKILL.md");
    fs::write(&path, SKILL_MD)
        .map_err(|error| Error::Config(format!("could not write {}: {error}", path.display())))?;
    Ok(path)
}
