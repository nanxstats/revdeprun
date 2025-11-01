use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

/// Describes the directories managed for a `revdeprun` invocation.
#[derive(Clone, Debug)]
pub struct Workspace {
    temp_dir: PathBuf,
    clone_root: PathBuf,
}

impl Workspace {
    /// Directory used for temporary files such as generated R scripts.
    pub fn temp_dir(&self) -> &Path {
        &self.temp_dir
    }

    /// Directory where remote repositories are cloned.
    pub fn clone_root(&self) -> &Path {
        &self.clone_root
    }
}

/// Prepares and returns the workspace directories used for cloning repositories
/// and storing temporary files.
///
/// When `custom` is `Some`, it is created if necessary and used both as the
/// clone root and temporary directory. Otherwise repositories are cloned into
/// the current working directory and temporary files are placed under
/// `./revdeprun-work`.
pub fn prepare(custom: Option<PathBuf>) -> Result<Workspace> {
    match custom {
        Some(path) => prepare_custom_workspace(path),
        None => prepare_default_workspace(),
    }
}

fn prepare_custom_workspace(path: PathBuf) -> Result<Workspace> {
    fs::create_dir_all(&path)
        .with_context(|| format!("failed to create custom workspace at {}", path.display()))?;

    Ok(Workspace {
        temp_dir: path.clone(),
        clone_root: path,
    })
}

fn prepare_default_workspace() -> Result<Workspace> {
    let clone_root = env::current_dir().context("failed to resolve current directory")?;
    let temp_dir = clone_root.join("revdeprun-work");
    fs::create_dir_all(&temp_dir)
        .with_context(|| format!("failed to create workspace at {}", temp_dir.display()))?;

    Ok(Workspace {
        temp_dir,
        clone_root,
    })
}

/// Returns the absolute path of `path` if it already exists.
///
/// This helper is used by modules that need to communicate user-facing paths.
pub fn canonicalized(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("failed to canonicalise {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn custom_workspace_uses_provided_path() {
        let tmp = tempdir().expect("tempdir");
        let base = tmp.path().join("workspace");
        let workspace = prepare(Some(base.clone())).expect("prepare custom workspace");

        assert_eq!(workspace.clone_root(), base.as_path());
        assert_eq!(workspace.temp_dir(), base.as_path());
        assert!(base.exists());
    }
}
