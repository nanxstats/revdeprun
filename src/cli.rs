use std::{num::NonZeroUsize, path::PathBuf};

use clap::{Parser, Subcommand};

/// Command-line arguments for the `revdeprun` CLI.
///
/// Running `revdeprun <REPOSITORY>` performs the end-to-end reverse dependency
/// check. Subcommands such as `revdeprun bundle` are mutually exclusive with
/// the check arguments.
#[derive(Debug, Parser)]
#[command(author, version, about = "Provision R and run reverse dependency check end-to-end", long_about = None)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Git URL, local directory, or source package tarball (.tar.gz) for the target R package.
    #[arg(required = true, value_name = "REPOSITORY")]
    pub repository: Option<String>,

    /// R version to install (e.g., release, 4.3.3, oldrel-1).
    #[arg(long = "r-version", default_value = "release")]
    pub r_version: String,

    /// Number of parallel workers for xfun::rev_check().
    #[arg(long, value_name = "N")]
    pub num_workers: Option<NonZeroUsize>,

    /// Optional workspace directory where temporary files are created.
    #[arg(long)]
    pub work_dir: Option<PathBuf>,

    /// Skip R version resolution and installation; reuse the system-wide installation.
    #[arg(long)]
    pub skip_r_install: bool,
}

/// Subcommands that run instead of the end-to-end check.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Bundle check results into a .tar.zst archive for transfer.
    Bundle(BundleArgs),
}

/// Arguments for `revdeprun bundle`.
#[derive(Debug, clap::Args)]
pub struct BundleArgs {
    /// Package directory where xfun::rev_check() ran (contains *.Rcheck/ and 00check_diffs.*).
    #[arg(default_value = ".", value_name = "PACKAGE_DIR")]
    pub package_dir: PathBuf,

    /// Output archive path (.tar.zst); an existing directory receives the default file name.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_check_arguments_without_subcommand() {
        let args = Args::try_parse_from([
            "revdeprun",
            "--r-version",
            "4.3.3",
            "--num-workers",
            "4",
            "https://github.com/user/pkg.git",
        ])
        .expect("check arguments");

        assert!(args.command.is_none());
        assert_eq!(
            args.repository.as_deref(),
            Some("https://github.com/user/pkg.git")
        );
        assert_eq!(args.r_version, "4.3.3");
        assert_eq!(args.num_workers.map(NonZeroUsize::get), Some(4));
    }

    #[test]
    fn requires_repository_without_subcommand() {
        assert!(Args::try_parse_from(["revdeprun"]).is_err());
        assert!(Args::try_parse_from(["revdeprun", "--r-version", "release"]).is_err());
    }

    #[test]
    fn parses_bundle_subcommand() {
        let args = Args::try_parse_from(["revdeprun", "bundle", "pkg", "--output", "out.tar.zst"])
            .expect("bundle arguments");

        assert!(args.repository.is_none());
        let Some(Command::Bundle(bundle)) = args.command else {
            panic!("expected the bundle subcommand");
        };
        assert_eq!(bundle.package_dir, PathBuf::from("pkg"));
        assert_eq!(bundle.output, Some(PathBuf::from("out.tar.zst")));
    }

    #[test]
    fn bundle_defaults_to_current_directory() {
        let args = Args::try_parse_from(["revdeprun", "bundle"]).expect("bundle arguments");

        let Some(Command::Bundle(bundle)) = args.command else {
            panic!("expected the bundle subcommand");
        };
        assert_eq!(bundle.package_dir, PathBuf::from("."));
        assert!(bundle.output.is_none());
    }

    #[test]
    fn bundle_rejects_check_options() {
        assert!(Args::try_parse_from(["revdeprun", "--skip-r-install", "bundle", "pkg"]).is_err());
        assert!(
            Args::try_parse_from(["revdeprun", "bundle", "pkg", "--r-version", "release"]).is_err()
        );
    }
}
