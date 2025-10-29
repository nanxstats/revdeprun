use std::{
    fs::File,
    io::copy,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use tempfile::TempDir;
use xshell::{Shell, cmd};

use crate::{progress::Progress, r_version::ResolvedRVersion};

/// Ensures the requested R toolchain is installed system-wide.
pub fn install_r(shell: &Shell, version: &ResolvedRVersion, progress: &Progress) -> Result<()> {
    let check_task = progress.task(format!(
        "Checking existing R {} installation",
        version.version
    ));
    if is_r_already_installed(shell, version)? {
        check_task.finish_with_message(format!("Using existing R {}", version.version));
        return Ok(());
    }
    check_task.finish_with_message(format!("R {} not detected; installing", version.version));

    let download_task = progress.task(format!("Downloading R {} installer", version.version));
    let installer = match download_installer(version) {
        Ok(installer) => {
            let file_name = installer
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("installer.deb");
            download_task
                .finish_with_message(format!("Downloaded R {} ({file_name})", version.version));
            installer
        }
        Err(err) => {
            download_task.fail(format!("Download of R {} failed", version.version));
            return Err(err);
        }
    };

    install_prerequisites(shell, progress).context("failed to install R prerequisites")?;
    install_from_deb(shell, installer.path(), progress)
        .with_context(|| format!("failed to install {}", installer.path().display()))?;
    configure_symlinks(shell, version, progress).context("failed to configure R symlinks")?;

    progress.println(format!("R {} installation completed", version.version));

    Ok(())
}

fn is_r_already_installed(shell: &Shell, version: &ResolvedRVersion) -> Result<bool> {
    let output = cmd!(shell, "R --version").ignore_status().read();
    Ok(match output {
        Ok(stdout) => stdout.contains(&version.version),
        Err(_) => false,
    })
}

fn install_prerequisites(shell: &Shell, progress: &Progress) -> Result<()> {
    run_command(
        progress,
        "Updating apt package metadata",
        "apt package metadata updated",
        cmd!(
            shell,
            "sudo env DEBIAN_FRONTEND=noninteractive apt-get update -y -qq"
        ),
    )?;

    run_command(
        progress,
        "Installing base R prerequisites",
        "base R prerequisites installed",
        cmd!(
            shell,
            "sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y gdebi-core qpdf devscripts ghostscript"
        ),
    )?;

    run_command(
        progress,
        "Installing revdep dependencies",
        "revdep dependencies installed",
        cmd!(
            shell,
            "sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y libcurl4-openssl-dev libssl-dev"
        ),
    )?;

    Ok(())
}

fn install_from_deb(shell: &Shell, package_path: &Path, progress: &Progress) -> Result<()> {
    let label = format!("Installing {}", package_path.display());
    run_command(
        progress,
        label.clone(),
        format!("Installed {}", package_path.display()),
        cmd!(shell, "sudo gdebi --non-interactive {package_path}"),
    )
}

fn configure_symlinks(
    shell: &Shell,
    version: &ResolvedRVersion,
    progress: &Progress,
) -> Result<()> {
    let install_dir = version.install_dir_name();
    let r_path = format!("/opt/R/{install_dir}/bin/R");
    let rscript_path = format!("/opt/R/{install_dir}/bin/Rscript");

    run_command(
        progress,
        "Linking R binary",
        format!("Linked /usr/local/bin/R -> {r_path}"),
        cmd!(shell, "sudo ln -sf {r_path} /usr/local/bin/R"),
    )?;

    run_command(
        progress,
        "Linking Rscript binary",
        format!("Linked /usr/local/bin/Rscript -> {rscript_path}"),
        cmd!(shell, "sudo ln -sf {rscript_path} /usr/local/bin/Rscript"),
    )?;
    Ok(())
}

fn run_command(
    progress: &Progress,
    start_message: impl Into<String>,
    success_message: impl Into<String>,
    command: xshell::Cmd<'_>,
) -> Result<()> {
    let start_message = start_message.into();
    let task = progress.task(start_message.clone());

    let output = match command.quiet().ignore_status().output() {
        Ok(output) => output,
        Err(err) => {
            task.fail(format!("{start_message} (failed to start)"));
            return Err(err.into());
        }
    };

    if output.status.success() {
        task.finish_with_message(success_message.into());
        return Ok(());
    }

    task.fail(format!("{start_message} (failed)"));
    emit_stream(progress, &start_message, "stdout", &output.stdout);
    emit_stream(progress, &start_message, "stderr", &output.stderr);

    bail!("{start_message} failed with status {}", output.status);
}

fn emit_stream(progress: &Progress, label: &str, stream_name: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    progress.println(format!("{label} {stream_name}:\n{trimmed}"));
}

struct DownloadedInstaller {
    #[allow(dead_code)]
    temp_dir: TempDir,
    path: PathBuf,
}

impl DownloadedInstaller {
    fn path(&self) -> &Path {
        &self.path
    }
}

fn download_installer(version: &ResolvedRVersion) -> Result<DownloadedInstaller> {
    let client = http_client()?;
    let response = client
        .get(version.url.clone())
        .send()
        .with_context(|| format!("failed to download {}", version.url))?
        .error_for_status()
        .with_context(|| format!("download returned error status for {}", version.url))?;

    let temp_dir = TempDir::new().context("failed to allocate temporary directory")?;
    let file_name = file_name_from_url(&version.url)?;
    let installer_path = temp_dir.path().join(file_name);

    let mut file = File::create(&installer_path)
        .with_context(|| format!("failed to create {}", installer_path.display()))?;
    let mut reader = response;
    copy(&mut reader, &mut file)
        .with_context(|| format!("failed to write {}", installer_path.display()))?;

    Ok(DownloadedInstaller {
        temp_dir,
        path: installer_path,
    })
}

fn http_client() -> Result<Client> {
    Client::builder()
        .user_agent(format!("revdeprun/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to construct HTTP client")
}

fn file_name_from_url(url: &str) -> Result<String> {
    let parsed =
        reqwest::Url::parse(url).with_context(|| format!("failed to parse download URL {url}"))?;
    parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
        .ok_or_else(|| anyhow::anyhow!("failed to extract file name from {url}"))
}
