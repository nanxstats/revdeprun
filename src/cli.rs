use std::{num::NonZeroUsize, path::PathBuf};

use clap::Parser;

/// Command-line arguments for the `revdeprun` CLI.
#[derive(Debug, Parser)]
#[command(author, version, about = "Provision R and run reverse dependency check end-to-end", long_about = None)]
pub struct Args {
    /// Git URL, local directory, or source package tarball (.tar.gz) for the target R package.
    pub repository: String,

    /// R version to install (e.g., release, 4.3.3, oldrel-1).
    #[arg(long = "r-version", default_value = "release")]
    pub r_version: String,

    /// Number of parallel workers for xfun::rev_check().
    #[arg(long, value_name = "N")]
    pub num_workers: Option<NonZeroUsize>,

    /// Optional workspace directory where temporary files are created.
    #[arg(long)]
    pub work_dir: Option<PathBuf>,

    /// Skip installing R and reuse the system-wide installation.
    #[arg(long)]
    pub skip_r_install: bool,
}
