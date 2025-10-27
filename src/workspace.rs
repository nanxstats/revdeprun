use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

/// Prepares and returns the workspace directory used for cloning repositories
/// and storing temporary files.
///
/// When `custom` is `Some`, the directory is created if necessary and returned
/// as-is. Otherwise a new directory under `./revdeprun-work` is created for
/// the current run.
pub fn prepare(custom: Option<PathBuf>) -> Result<PathBuf> {
    match custom {
        Some(path) => {
            fs::create_dir_all(&path).with_context(|| {
                format!("failed to create custom workspace at {}", path.display())
            })?;
            Ok(path)
        }
        None => default_workspace(),
    }
}

fn default_workspace() -> Result<PathBuf> {
    let base = env::current_dir()
        .context("failed to resolve current directory")?
        .join("revdeprun-work");
    fs::create_dir_all(&base)
        .with_context(|| format!("failed to create workspace base at {}", base.display()))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for attempt in 0u32..100 {
        let candidate = base.join(format!("run-{timestamp}-{attempt}"));
        match fs::create_dir(&candidate) {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to create workspace directory at {}",
                        candidate.display()
                    )
                });
            }
        }
    }

    Err(anyhow::anyhow!(
        "failed to allocate a unique workspace directory under {}",
        base.display()
    ))
}

/// Returns the absolute path of `path` if it already exists.
///
/// This helper is used by modules that need to communicate user-facing paths.
pub fn canonicalized(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("failed to canonicalise {}", path.display()))
}
