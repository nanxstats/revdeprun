//! Bundles the files left behind by `xfun::rev_check()` into a `.tar.zst`
//! archive so results can be moved off a cloud instance for review.
//!
//! `xfun::rev_check()` works inside the package directory and, per the xfun
//! sources (`R/revcheck.R`), leaves behind exactly these review artifacts:
//!
//! - `00check_diffs.md` and `00check_diffs.html`: the summary written by
//!   `xfun::compare_Rcheck()` for reverse dependencies whose check results
//!   differ between the CRAN and development versions of the package.
//! - `<pkg>.Rcheck/`: `R CMD check` output for a reverse dependency checked
//!   against the CRAN version of the package.
//! - `<pkg>.Rcheck2/`: the same check against the development version.
//!
//! Successful checks delete their `*.Rcheck` directory, so these entries only
//! exist for reverse dependencies with problems. Everything else in the
//! package directory (the package sources, `revdep/library/`, downloaded
//! `tarball/` sources, and the transient `library-cran/`) stays out of the
//! bundle.

use std::{
    env, fs,
    io::{BufWriter, Write},
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use tempfile::NamedTempFile;

use crate::{cli::BundleArgs, progress::Progress, util, workspace};

/// File extension of bundles written by this module.
pub const BUNDLE_EXTENSION: &str = "tar.zst";

/// Summary reports written by `xfun::compare_Rcheck()`.
const REPORT_FILES: [&str; 2] = ["00check_diffs.md", "00check_diffs.html"];

/// zstd compression level for bundles.
///
/// Check logs and package sources are highly compressible text. Level 9 makes
/// archives noticeably smaller than the zstd default (3) while still
/// compressing at tens of megabytes per second per core, and compression runs
/// on all available cores.
const COMPRESSION_LEVEL: i32 = 9;

/// Placeholder printed in the `scp` hint when the instance address is unknown.
const HOST_PLACEHOLDER: &str = "HOST";

/// A file or directory produced by `xfun::rev_check()` in the package directory.
///
/// Variants are ordered so that sorting places reports before check
/// directories, each group alphabetically.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Artifact {
    /// A summary report file (`00check_diffs.md` or `00check_diffs.html`).
    Report(String),
    /// A `*.Rcheck` or `*.Rcheck2` directory.
    CheckDir(String),
}

impl Artifact {
    /// File or directory name relative to the package directory.
    pub fn name(&self) -> &str {
        match self {
            Artifact::Report(name) | Artifact::CheckDir(name) => name,
        }
    }
}

/// Size statistics for a written bundle.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BundleStats {
    /// Number of regular files added to the archive.
    pub files: usize,
    /// Total size of the archived files before compression.
    pub uncompressed_bytes: u64,
    /// Size of the finished `.tar.zst` file.
    pub compressed_bytes: u64,
}

/// Runs the `revdeprun bundle` subcommand.
pub fn run(args: &BundleArgs, progress: &Progress) -> Result<()> {
    let package_dir = workspace::canonicalized(&args.package_dir).with_context(|| {
        format!(
            "failed to resolve package directory {}",
            args.package_dir.display()
        )
    })?;
    if !package_dir.is_dir() {
        bail!("{} is not a directory", package_dir.display());
    }

    let artifacts = collect_artifacts(&package_dir)?;
    if artifacts.is_empty() {
        bail!(
            "no xfun::rev_check() results found in {}: expected 00check_diffs.md, \
             00check_diffs.html, or *.Rcheck/ directories. Either the check reported no \
             problems to review, or this is not the package directory the check ran in.",
            package_dir.display()
        );
    }

    let package_name = package_name(&package_dir);
    let output = resolve_output_path(args.output.as_deref(), &package_dir, &package_name)?;
    if output.exists() {
        bail!(
            "refusing to overwrite existing file {}; remove it or choose a different --output",
            output.display()
        );
    }
    let prefix = archive_prefix(&output, &package_name);

    let task = progress.task(format!(
        "Bundling xfun::rev_check() results from {}",
        package_dir.display()
    ));
    let stats = match write_bundle(
        &package_dir,
        &artifacts,
        &output,
        &prefix,
        available_parallelism(),
    ) {
        Ok(stats) => stats,
        Err(err) => {
            task.fail(format!(
                "Failed to bundle results from {}",
                package_dir.display()
            ));
            return Err(err);
        }
    };
    let output = workspace::canonicalized(&output).unwrap_or(output);
    task.finish_with_message(format!("Bundle written to {}", output.display()));

    let user = env::var("USER")
        .or_else(|_| env::var("LOGNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "USER".to_string());
    let host = env::var("SSH_CONNECTION")
        .ok()
        .as_deref()
        .and_then(public_host_from_ssh_connection)
        .unwrap_or_else(|| HOST_PLACEHOLDER.to_string());

    progress.println(summary(&output, &artifacts, &stats, &user, &host));
    Ok(())
}

/// Returns whether `package_dir` contains any `xfun::rev_check()` results.
pub fn has_results(package_dir: &Path) -> bool {
    collect_artifacts(package_dir).is_ok_and(|artifacts| !artifacts.is_empty())
}

/// Lists the `xfun::rev_check()` artifacts directly under `package_dir`,
/// sorted with report files first and check directories second.
pub fn collect_artifacts(package_dir: &Path) -> Result<Vec<Artifact>> {
    let entries = fs::read_dir(package_dir)
        .with_context(|| format!("failed to read directory {}", package_dir.display()))?;

    let mut artifacts = Vec::new();
    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read directory {}", package_dir.display()))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let path = entry.path();
        if REPORT_FILES.contains(&name.as_str()) && path.is_file() {
            artifacts.push(Artifact::Report(name));
        } else if is_check_dir_name(&name) && path.is_dir() {
            artifacts.push(Artifact::CheckDir(name));
        }
    }
    artifacts.sort();
    Ok(artifacts)
}

/// Matches the `.+[.]Rcheck$` and `.+[.]Rcheck2$` patterns used by xfun.
fn is_check_dir_name(name: &str) -> bool {
    ["Rcheck", "Rcheck2"].iter().any(|suffix| {
        name.strip_suffix(suffix)
            .and_then(|stem| stem.strip_suffix('.'))
            .is_some_and(|stem| !stem.is_empty())
    })
}

/// Reads the package name from `DESCRIPTION`, falling back to the directory name.
fn package_name(package_dir: &Path) -> String {
    fs::read_to_string(package_dir.join("DESCRIPTION"))
        .ok()
        .and_then(|contents| description_package_field(&contents))
        .or_else(|| {
            package_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "package".to_string())
}

fn description_package_field(description: &str) -> Option<String> {
    description
        .lines()
        .find_map(|line| line.strip_prefix("Package:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn default_file_name(package_name: &str) -> String {
    format!("{package_name}-revdep.{BUNDLE_EXTENSION}")
}

/// Resolves where the bundle is written.
///
/// Without `--output`, the bundle is placed next to the package directory so
/// it never ends up inside the package sources. An existing directory passed
/// as `--output` receives the default file name.
fn resolve_output_path(
    requested: Option<&Path>,
    package_dir: &Path,
    package_name: &str,
) -> Result<PathBuf> {
    match requested {
        Some(path) if path.is_dir() => Ok(path.join(default_file_name(package_name))),
        Some(path) => Ok(path.to_path_buf()),
        None => {
            let parent = package_dir.parent().ok_or_else(|| {
                anyhow!(
                    "cannot determine the parent directory of {}",
                    package_dir.display()
                )
            })?;
            Ok(parent.join(default_file_name(package_name)))
        }
    }
}

/// Name of the single top-level directory inside the archive, derived from the
/// output file name so extracting the bundle never scatters files.
fn archive_prefix(output: &Path, package_name: &str) -> String {
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let stem = file_name
        .strip_suffix(&format!(".{BUNDLE_EXTENSION}"))
        .or_else(|| {
            Path::new(file_name)
                .file_stem()
                .and_then(|stem| stem.to_str())
        })
        .unwrap_or_default();
    if stem.is_empty() || stem == "." || stem == ".." {
        format!("{package_name}-revdep")
    } else {
        stem.to_owned()
    }
}

/// Writes `artifacts` from `package_dir` into a zstd-compressed tar archive at
/// `output`, with every entry placed under `prefix/`.
///
/// The archive is assembled in a temporary file next to `output` and moved
/// into place only once it is complete, so a failure never leaves a partial
/// bundle behind. The output directory must be outside the collected check
/// directories so traversal cannot pick up the growing temporary file.
fn write_bundle(
    package_dir: &Path,
    artifacts: &[Artifact],
    output: &Path,
    prefix: &str,
    workers: u32,
) -> Result<BundleStats> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = workspace::canonicalized(parent)
        .with_context(|| format!("failed to resolve output directory {}", parent.display()))?;
    // Resolve both sides so symlink aliases and `..` cannot bypass this check.
    for artifact in artifacts {
        if let Artifact::CheckDir(name) = artifact {
            let source = workspace::canonicalized(&package_dir.join(name))?;
            if parent.starts_with(&source) {
                bail!(
                    "output directory {} is inside check results directory {}; \
                     choose an --output outside the check result directories",
                    parent.display(),
                    source.display()
                );
            }
        }
    }
    let temp = bundle_tempfile(&parent)
        .with_context(|| format!("failed to create a temporary file in {}", parent.display()))?;

    let mut stats = BundleStats::default();
    {
        let mut encoder = zstd::Encoder::new(BufWriter::new(temp.as_file()), COMPRESSION_LEVEL)
            .context("failed to initialize zstd compression")?;
        encoder
            .multithread(workers)
            .context("failed to configure zstd worker threads")?;

        let mut builder = tar::Builder::new(encoder);
        builder.follow_symlinks(true);
        builder
            .append_dir(prefix, package_dir)
            .with_context(|| format!("failed to add {prefix}/ to the bundle"))?;
        for artifact in artifacts {
            let source = package_dir.join(artifact.name());
            let target = Path::new(prefix).join(artifact.name());
            match artifact {
                Artifact::Report(_) => append_file(&mut builder, &source, &target, &mut stats)?,
                Artifact::CheckDir(_) => {
                    append_dir_recursive(&mut builder, &source, &target, &mut stats)?
                }
            }
        }

        let encoder = builder
            .into_inner()
            .context("failed to finalize the tar archive")?;
        let mut writer = encoder
            .finish()
            .context("failed to finalize the zstd stream")?;
        writer
            .flush()
            .context("failed to flush the bundle to disk")?;
    }

    temp.as_file()
        .sync_all()
        .context("failed to sync the bundle to disk")?;
    stats.compressed_bytes = temp
        .as_file()
        .metadata()
        .context("failed to read bundle size")?
        .len();
    temp.persist_noclobber(output).with_context(|| {
        format!(
            "failed to move the bundle into place at {}",
            output.display()
        )
    })?;

    Ok(stats)
}

/// Creates the temporary file the bundle is assembled in.
///
/// Temporary files default to owner-only permissions; the bundle is an ordinary
/// output file, so request the usual `0644` (subject to the umask) on Unix.
fn bundle_tempfile(parent: &Path) -> std::io::Result<NamedTempFile> {
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o644));
    }
    builder.tempfile_in(parent)
}

/// Appends `source` and its contents to the archive under `target`, visiting
/// entries in sorted order so bundles are reproducible.
fn append_dir_recursive<W: Write>(
    builder: &mut tar::Builder<W>,
    source: &Path,
    target: &Path,
    stats: &mut BundleStats,
) -> Result<()> {
    builder
        .append_dir(target, source)
        .with_context(|| format!("failed to add directory {} to the bundle", source.display()))?;

    let mut entries = fs::read_dir(source)
        .and_then(|entries| entries.collect::<std::io::Result<Vec<_>>>())
        .with_context(|| format!("failed to read directory {}", source.display()))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let child_target = target.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if file_type.is_dir() {
            append_dir_recursive(builder, &path, &child_target, stats)?;
        } else if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
            append_file(builder, &path, &child_target, stats)?;
        }
        // Symlinked directories, broken links, and special files are skipped.
    }
    Ok(())
}

fn append_file<W: Write>(
    builder: &mut tar::Builder<W>,
    source: &Path,
    target: &Path,
    stats: &mut BundleStats,
) -> Result<()> {
    let metadata =
        fs::metadata(source).with_context(|| format!("failed to inspect {}", source.display()))?;
    builder
        .append_path_with_name(source, target)
        .with_context(|| format!("failed to add file {} to the bundle", source.display()))?;
    stats.files += 1;
    stats.uncompressed_bytes += metadata.len();
    Ok(())
}

fn available_parallelism() -> u32 {
    std::thread::available_parallelism()
        .map(|count| u32::try_from(count.get()).unwrap_or(u32::MAX))
        .unwrap_or(1)
}

/// Builds the completion message, including copy-and-paste `scp` and `tar`
/// commands for transferring and extracting the bundle.
fn summary(
    output: &Path,
    artifacts: &[Artifact],
    stats: &BundleStats,
    user: &str,
    host: &str,
) -> String {
    let file_name = output
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| output.display().to_string());
    format!(
        "Reverse dependency check results bundled successfully.\n  \
         • bundle: {} ({}, compressed from {})\n  \
         • contents: {} ({} files)\n\
         Copy the bundle to another machine and extract it with:\n  \
         {}\n  \
         tar -xf {}",
        output.display(),
        format_size(stats.compressed_bytes),
        format_size(stats.uncompressed_bytes),
        describe_artifacts(artifacts),
        stats.files,
        scp_command(user, host, output),
        util::shell_quote(&file_name)
    )
}

/// Summarizes the bundled artifacts, for example
/// `00check_diffs.html, 00check_diffs.md, 3 *.Rcheck/, 2 *.Rcheck2/`.
fn describe_artifacts(artifacts: &[Artifact]) -> String {
    let mut parts: Vec<String> = artifacts
        .iter()
        .filter_map(|artifact| match artifact {
            Artifact::Report(name) => Some(name.clone()),
            Artifact::CheckDir(_) => None,
        })
        .collect();
    for suffix in [".Rcheck", ".Rcheck2"] {
        let count = artifacts
            .iter()
            .filter(
                |artifact| matches!(artifact, Artifact::CheckDir(name) if name.ends_with(suffix)),
            )
            .count();
        if count > 0 {
            parts.push(format!("{count} *{suffix}/"));
        }
    }
    parts.join(", ")
}

/// Renders the `scp` command that copies `remote_path` from this machine.
fn scp_command(user: &str, host: &str, remote_path: &Path) -> String {
    let source = format!("{user}@{host}:{}", remote_path.display());
    format!("scp {} .", util::shell_quote(&source))
}

/// Extracts the address clients used to reach this machine from sshd's
/// `SSH_CONNECTION` value (`client_ip client_port server_ip server_port`).
///
/// Private, loopback, and link-local server addresses are rejected because
/// cloud instances behind NAT only see their internal address, which would
/// make the printed `scp` command wrong. IPv6 addresses are bracketed for scp.
fn public_host_from_ssh_connection(value: &str) -> Option<String> {
    let server = value.split_whitespace().nth(2)?;
    match server.parse::<IpAddr>().ok()? {
        IpAddr::V4(addr) if is_public_ipv4(addr) => Some(server.to_owned()),
        IpAddr::V6(addr)
            if !(addr.is_loopback()
                || addr.is_unspecified()
                || addr.is_unique_local()
                || addr.is_unicast_link_local()) =>
        {
            Some(format!("[{server}]"))
        }
        _ => None,
    }
}

fn is_public_ipv4(addr: Ipv4Addr) -> bool {
    let octets = addr.octets();
    let carrier_grade_nat = octets[0] == 100 && (64..=127).contains(&octets[1]);
    !(addr.is_private()
        || addr.is_loopback()
        || addr.is_link_local()
        || addr.is_unspecified()
        || addr.is_broadcast()
        || addr.is_documentation()
        || carrier_grade_nat)
}

/// Formats a byte count with binary units, for example `1.5 MiB`.
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::tempdir;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, contents).expect("write file");
    }

    /// Creates a package directory resembling one after `xfun::rev_check()`
    /// ran, including files that must stay out of the bundle.
    fn sample_package_dir(root: &Path) -> PathBuf {
        let pkg = root.join("ggsci");
        write(&pkg.join("DESCRIPTION"), "Package: ggsci\nVersion: 4.0.0\n");
        write(&pkg.join("R/zzz.R"), "NULL\n");
        write(
            &pkg.join("00check_diffs.md"),
            "---\ntitle: xfun::rev_check() results\n---\n",
        );
        write(&pkg.join("00check_diffs.html"), "<html></html>\n");
        write(
            &pkg.join("alpha.Rcheck/00check.log"),
            "* checking examples ... WARNING\n",
        );
        write(&pkg.join("alpha.Rcheck/00install.log"), "install\n");
        write(
            &pkg.join("alpha.Rcheck/tests/testthat.Rout.fail"),
            "Error\n",
        );
        write(&pkg.join("alpha.Rcheck2/00check.log"), "Status: OK\n");
        write(&pkg.join("beta.Rcheck/00check.log"), "Status: 1 ERROR\n");
        fs::create_dir_all(pkg.join("beta.Rcheck/00_pkg_src/beta")).expect("empty source dir");
        // Not results: the revdep library, downloaded tarballs, the transient
        // CRAN library, and look-alike names.
        write(
            &pkg.join("revdep/library/xfun/DESCRIPTION"),
            "Package: xfun\n",
        );
        write(&pkg.join("tarball/alpha_1.0.0.tar.gz"), "gz");
        fs::create_dir_all(pkg.join("library-cran")).expect("library-cran");
        write(&pkg.join("gamma.Rcheck"), "a file, not a directory");
        fs::create_dir_all(pkg.join(".Rcheck")).expect("dot Rcheck");
        fs::create_dir_all(pkg.join("delta.Rcheck3")).expect("Rcheck3");
        pkg
    }

    fn read_bundle(path: &Path) -> Vec<(String, tar::EntryType, String)> {
        let file = fs::File::open(path).expect("open bundle");
        let decoder = zstd::Decoder::new(file).expect("zstd decoder");
        let mut archive = tar::Archive::new(decoder);
        archive
            .entries()
            .expect("entries")
            .map(|entry| {
                let mut entry = entry.expect("entry");
                let path = entry
                    .path()
                    .expect("entry path")
                    .to_string_lossy()
                    .trim_end_matches('/')
                    .to_string();
                let kind = entry.header().entry_type();
                let mut contents = String::new();
                entry.read_to_string(&mut contents).expect("entry contents");
                (path, kind, contents)
            })
            .collect()
    }

    #[test]
    fn collects_reports_then_check_directories_in_order() {
        let tmp = tempdir().expect("tempdir");
        let pkg = sample_package_dir(tmp.path());

        let artifacts = collect_artifacts(&pkg).expect("artifacts");
        let names: Vec<&str> = artifacts.iter().map(Artifact::name).collect();

        assert_eq!(
            names,
            [
                "00check_diffs.html",
                "00check_diffs.md",
                "alpha.Rcheck",
                "alpha.Rcheck2",
                "beta.Rcheck",
            ]
        );
        assert!(matches!(artifacts[0], Artifact::Report(_)));
        assert!(matches!(artifacts[2], Artifact::CheckDir(_)));
        assert!(has_results(&pkg));
    }

    #[test]
    fn directories_without_results_have_none() {
        let tmp = tempdir().expect("tempdir");
        write(&tmp.path().join("DESCRIPTION"), "Package: clean\n");
        write(
            &tmp.path().join("revdep/library/x/DESCRIPTION"),
            "Package: x\n",
        );

        assert!(collect_artifacts(tmp.path()).expect("artifacts").is_empty());
        assert!(!has_results(tmp.path()));
        assert!(!has_results(&tmp.path().join("missing")));
    }

    #[test]
    fn matches_xfun_check_directory_patterns() {
        for name in ["pkg.Rcheck", "pkg.Rcheck2", "data.table.Rcheck", "a.Rcheck"] {
            assert!(is_check_dir_name(name), "{name} should match");
        }
        for name in [
            ".Rcheck",
            ".Rcheck2",
            "Rcheck",
            "pkg.Rcheck3",
            "pkg.rcheck",
            "pkgRcheck",
            "pkg.Rcheck/",
        ] {
            assert!(!is_check_dir_name(name), "{name} should not match");
        }
    }

    #[test]
    fn reads_package_name_from_description_or_directory() {
        let tmp = tempdir().expect("tempdir");
        let pkg = tmp.path().join("checkout");
        write(
            &pkg.join("DESCRIPTION"),
            "Type: Package\nPackage:   ggsci  \nVersion: 1.0\n",
        );
        assert_eq!(package_name(&pkg), "ggsci");

        let bare = tmp.path().join("bare");
        fs::create_dir_all(&bare).expect("bare dir");
        assert_eq!(package_name(&bare), "bare");

        assert_eq!(description_package_field("Version: 1.0\n"), None);
        assert_eq!(description_package_field("Package:\n"), None);
    }

    #[test]
    fn resolves_output_path_next_to_package_or_as_requested() {
        let tmp = tempdir().expect("tempdir");
        let pkg = tmp.path().join("ggsci");
        fs::create_dir_all(&pkg).expect("package dir");

        assert_eq!(
            resolve_output_path(None, &pkg, "ggsci").expect("default output"),
            tmp.path().join("ggsci-revdep.tar.zst")
        );
        assert_eq!(
            resolve_output_path(Some(Path::new("/data/out.tar.zst")), &pkg, "ggsci")
                .expect("explicit output"),
            PathBuf::from("/data/out.tar.zst")
        );
        assert_eq!(
            resolve_output_path(Some(tmp.path()), &pkg, "ggsci").expect("directory output"),
            tmp.path().join("ggsci-revdep.tar.zst")
        );
    }

    #[test]
    fn derives_archive_prefix_from_output_name() {
        assert_eq!(
            archive_prefix(Path::new("/x/ggsci-revdep.tar.zst"), "ggsci"),
            "ggsci-revdep"
        );
        assert_eq!(
            archive_prefix(Path::new("round2.tar.zst"), "ggsci"),
            "round2"
        );
        assert_eq!(archive_prefix(Path::new("results.zst"), "ggsci"), "results");
        assert_eq!(archive_prefix(Path::new("."), "ggsci"), "ggsci-revdep");
    }

    #[test]
    fn writes_bundle_containing_only_results() {
        let tmp = tempdir().expect("tempdir");
        let pkg = sample_package_dir(tmp.path());
        let artifacts = collect_artifacts(&pkg).expect("artifacts");
        let output = tmp.path().join("ggsci-revdep.tar.zst");

        let stats =
            write_bundle(&pkg, &artifacts, &output, "ggsci-revdep", 2).expect("write bundle");

        assert_eq!(stats.files, 7);
        assert_eq!(
            stats.uncompressed_bytes,
            [
                "<html></html>\n",
                "---\ntitle: xfun::rev_check() results\n---\n",
                "* checking examples ... WARNING\n",
                "install\n",
                "Error\n",
                "Status: OK\n",
                "Status: 1 ERROR\n",
            ]
            .iter()
            .map(|contents| contents.len() as u64)
            .sum::<u64>()
        );
        assert_eq!(
            stats.compressed_bytes,
            fs::metadata(&output).expect("bundle metadata").len()
        );
        assert!(stats.compressed_bytes > 0);

        let entries = read_bundle(&output);
        let paths: Vec<&str> = entries.iter().map(|(path, _, _)| path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "ggsci-revdep",
                "ggsci-revdep/00check_diffs.html",
                "ggsci-revdep/00check_diffs.md",
                "ggsci-revdep/alpha.Rcheck",
                "ggsci-revdep/alpha.Rcheck/00check.log",
                "ggsci-revdep/alpha.Rcheck/00install.log",
                "ggsci-revdep/alpha.Rcheck/tests",
                "ggsci-revdep/alpha.Rcheck/tests/testthat.Rout.fail",
                "ggsci-revdep/alpha.Rcheck2",
                "ggsci-revdep/alpha.Rcheck2/00check.log",
                "ggsci-revdep/beta.Rcheck",
                "ggsci-revdep/beta.Rcheck/00_pkg_src",
                "ggsci-revdep/beta.Rcheck/00_pkg_src/beta",
                "ggsci-revdep/beta.Rcheck/00check.log",
            ]
        );
        assert!(entries.iter().all(|(path, _, _)| {
            !path.contains("revdep/library")
                && !path.contains("tarball")
                && !path.contains("library-cran")
                && !path.contains("DESCRIPTION")
                && !path.contains("gamma")
                && !path.contains("delta")
        }));

        let (_, kind, contents) = &entries[4];
        assert_eq!(*kind, tar::EntryType::Regular);
        assert_eq!(contents, "* checking examples ... WARNING\n");
        let (_, kind, _) = &entries[3];
        assert_eq!(*kind, tar::EntryType::Directory);

        // Only the package directory and the bundle remain next to each other.
        let mut siblings: Vec<String> = fs::read_dir(tmp.path())
            .expect("read parent")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        siblings.sort();
        assert_eq!(siblings, ["ggsci", "ggsci-revdep.tar.zst"]);
    }

    #[test]
    fn rejects_output_inside_check_directories() {
        let tmp = tempdir().expect("tempdir");
        let pkg = sample_package_dir(tmp.path());
        let artifacts = collect_artifacts(&pkg).expect("artifacts");

        for relative in [
            "alpha.Rcheck/results.tar.zst",
            "alpha.Rcheck/tests/results.tar.zst",
            "alpha.Rcheck2/results.tar.zst",
            "alpha.Rcheck/tests/../results.tar.zst",
        ] {
            let output = pkg.join(relative);
            let parent = output.parent().expect("parent");
            let entries_before = fs::read_dir(parent).expect("read parent").count();
            let err = write_bundle(&pkg, &artifacts, &output, "results", 1)
                .expect_err("output inside check results must fail");

            assert!(
                err.to_string().contains("inside check results directory"),
                "{err:#}"
            );
            assert!(!output.exists());
            assert_eq!(
                fs::read_dir(parent).expect("read parent").count(),
                entries_before
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_output_through_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let tmp = tempdir().expect("tempdir");
        let pkg = sample_package_dir(tmp.path());
        let output_alias = tmp.path().join("output-alias");
        symlink(pkg.join("alpha.Rcheck/tests"), &output_alias).expect("output symlink");

        let external = tmp.path().join("external");
        fs::create_dir(&external).expect("external directory");
        symlink(&external, pkg.join("external.Rcheck")).expect("artifact symlink");
        let artifacts = collect_artifacts(&pkg).expect("artifacts");

        for parent in [output_alias, external] {
            let output = parent.join("results.tar.zst");
            let entries_before = fs::read_dir(&parent).expect("read parent").count();
            let err = write_bundle(&pkg, &artifacts, &output, "results", 1)
                .expect_err("symlinked output inside check results must fail");

            assert!(
                err.to_string().contains("inside check results directory"),
                "{err:#}"
            );
            assert!(!output.exists());
            assert_eq!(
                fs::read_dir(&parent).expect("read parent").count(),
                entries_before
            );
        }
    }

    #[test]
    fn writes_bundle_inside_package_outside_check_directories() {
        let tmp = tempdir().expect("tempdir");
        let pkg = sample_package_dir(tmp.path());
        fs::create_dir(pkg.join("alpha.Rcheck-backup")).expect("non-result directory");
        let artifacts = collect_artifacts(&pkg).expect("artifacts");

        for relative in [
            "results.tar.zst",
            "alpha.Rcheck-backup/results.tar.zst",
            "alpha.Rcheck/tests/../../normalized.tar.zst",
        ] {
            let output = pkg.join(relative);
            let stats = write_bundle(&pkg, &artifacts, &output, "results", 1)
                .expect("output outside check directories");

            assert_eq!(stats.files, 7);
            assert_eq!(read_bundle(&output).len(), 14);
        }
    }

    #[test]
    fn run_writes_default_bundle_and_refuses_to_overwrite() {
        let tmp = tempdir().expect("tempdir");
        let pkg = sample_package_dir(tmp.path());
        let progress = Progress::new();
        let args = BundleArgs {
            package_dir: pkg.clone(),
            output: None,
        };

        run(&args, &progress).expect("first bundle");
        let output = tmp.path().join("ggsci-revdep.tar.zst");
        assert!(output.is_file());
        assert!(!read_bundle(&output).is_empty());

        let err = run(&args, &progress).expect_err("second bundle must fail");
        assert!(err.to_string().contains("refusing to overwrite"), "{err}");
    }

    #[test]
    fn run_fails_without_results() {
        let tmp = tempdir().expect("tempdir");
        let pkg = tmp.path().join("clean");
        write(&pkg.join("DESCRIPTION"), "Package: clean\n");
        let progress = Progress::new();
        let args = BundleArgs {
            package_dir: pkg,
            output: Some(tmp.path().join("never.tar.zst")),
        };

        let err = run(&args, &progress).expect_err("nothing to bundle");
        assert!(
            err.to_string()
                .contains("no xfun::rev_check() results found"),
            "{err}"
        );
        assert!(!tmp.path().join("never.tar.zst").exists());
    }

    #[test]
    fn describes_bundled_artifacts() {
        let artifacts = vec![
            Artifact::Report("00check_diffs.html".into()),
            Artifact::Report("00check_diffs.md".into()),
            Artifact::CheckDir("a.Rcheck".into()),
            Artifact::CheckDir("a.Rcheck2".into()),
            Artifact::CheckDir("b.Rcheck".into()),
        ];
        assert_eq!(
            describe_artifacts(&artifacts),
            "00check_diffs.html, 00check_diffs.md, 2 *.Rcheck/, 1 *.Rcheck2/"
        );
        assert_eq!(
            describe_artifacts(&[Artifact::CheckDir("a.Rcheck".into())]),
            "1 *.Rcheck/"
        );
    }

    #[test]
    fn renders_scp_command() {
        assert_eq!(
            scp_command(
                "ubuntu",
                "203.0.113.10",
                Path::new("/home/ubuntu/ggsci-revdep.tar.zst")
            ),
            "scp ubuntu@203.0.113.10:/home/ubuntu/ggsci-revdep.tar.zst ."
        );
        assert_eq!(
            scp_command(
                "ubuntu",
                HOST_PLACEHOLDER,
                Path::new("/data/my pkg/out.tar.zst")
            ),
            "scp 'ubuntu@HOST:/data/my pkg/out.tar.zst' ."
        );
    }

    #[test]
    fn detects_public_server_address_from_ssh_connection() {
        assert_eq!(
            public_host_from_ssh_connection("198.51.100.7 51234 93.184.216.34 22"),
            Some("93.184.216.34".to_string())
        );
        assert_eq!(
            public_host_from_ssh_connection("198.51.100.7 51234 2a00:1450:4001:80b::200e 22"),
            Some("[2a00:1450:4001:80b::200e]".to_string())
        );
        for value in [
            "198.51.100.7 51234 172.31.4.2 22",
            "198.51.100.7 51234 10.0.0.5 22",
            "198.51.100.7 51234 192.168.1.9 22",
            "198.51.100.7 51234 100.64.0.1 22",
            "198.51.100.7 51234 169.254.169.254 22",
            "198.51.100.7 51234 127.0.0.1 22",
            "198.51.100.7 51234 fe80::1 22",
            "198.51.100.7 51234 fd12:3456::1 22",
            "198.51.100.7 51234 ::1 22",
            "198.51.100.7 51234 not-an-ip 22",
            "198.51.100.7 51234",
            "",
        ] {
            assert_eq!(public_host_from_ssh_connection(value), None, "{value:?}");
        }
    }

    #[test]
    fn formats_sizes_with_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1_572_864), "1.5 MiB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn summary_lists_bundle_transfer_commands() {
        let artifacts = vec![
            Artifact::Report("00check_diffs.md".into()),
            Artifact::CheckDir("a.Rcheck".into()),
        ];
        let stats = BundleStats {
            files: 3,
            uncompressed_bytes: 2048,
            compressed_bytes: 1024,
        };
        let text = summary(
            Path::new("/home/ubuntu/ggsci-revdep.tar.zst"),
            &artifacts,
            &stats,
            "ubuntu",
            HOST_PLACEHOLDER,
        );

        assert!(text.contains(
            "bundle: /home/ubuntu/ggsci-revdep.tar.zst (1.0 KiB, compressed from 2.0 KiB)"
        ));
        assert!(text.contains("contents: 00check_diffs.md, 1 *.Rcheck/ (3 files)"));
        assert!(text.contains("\n  scp ubuntu@HOST:/home/ubuntu/ggsci-revdep.tar.zst .\n"));
        assert!(text.ends_with("tar -xf ggsci-revdep.tar.zst"));
    }
}
