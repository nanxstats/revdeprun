use std::{
    fs::File,
    io::copy,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use tempfile::TempDir;
use xshell::{Shell, cmd};

use crate::r_version::ResolvedRVersion;

/// Ensures the requested R toolchain is installed system-wide.
pub fn install_r(shell: &Shell, version: &ResolvedRVersion) -> Result<()> {
    if is_r_already_installed(shell, version)? {
        println!(
            "R {} already detected on PATH; skipping installation.",
            version.version
        );
        return Ok(());
    }

    println!("Installing R {} from {}", version.version, version.url);
    let installer = download_installer(version)?;
    install_prerequisites(shell).context("failed to install R prerequisites")?;
    install_from_deb(shell, installer.path()).context("failed to install downloaded R package")?;
    configure_symlinks(shell, version).context("failed to configure R symlinks")?;
    println!("R {} installation completed.", version.version);

    Ok(())
}

fn is_r_already_installed(shell: &Shell, version: &ResolvedRVersion) -> Result<bool> {
    let output = cmd!(shell, "R --version").ignore_status().read();
    Ok(match output {
        Ok(stdout) => stdout.contains(&version.version),
        Err(_) => false,
    })
}

fn install_prerequisites(shell: &Shell) -> Result<()> {
    cmd!(
        shell,
        "sudo env DEBIAN_FRONTEND=noninteractive apt-get update -y -qq"
    )
    .run()
    .context("apt-get update failed")?;

    cmd!(
        shell,
        "sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y gdebi-core qpdf devscripts ghostscript"
    )
    .run()
    .context("apt-get install failed")?;

    Ok(())
}

fn install_from_deb(shell: &Shell, package_path: &Path) -> Result<()> {
    cmd!(shell, "sudo gdebi --non-interactive {package_path}")
        .run()
        .with_context(|| format!("gdebi failed to install {}", package_path.display()))
}

fn configure_symlinks(shell: &Shell, version: &ResolvedRVersion) -> Result<()> {
    let install_dir = version.install_dir_name();
    let r_path = format!("/opt/R/{install_dir}/bin/R");
    let rscript_path = format!("/opt/R/{install_dir}/bin/Rscript");

    cmd!(shell, "sudo ln -sf {r_path} /usr/local/bin/R")
        .run()
        .context("failed to symlink R binary")?;
    cmd!(shell, "sudo ln -sf {rscript_path} /usr/local/bin/Rscript")
        .run()
        .context("failed to symlink Rscript binary")?;
    Ok(())
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
        .and_then(|segments| segments.last())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
        .ok_or_else(|| anyhow::anyhow!("failed to extract file name from {url}"))
}
