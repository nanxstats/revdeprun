use std::{
    env,
    fs::File,
    io::copy,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use tempfile::TempDir;
use xshell::{Shell, cmd};

use crate::{progress::Progress, r_version::ResolvedRVersion};

const QUARTO_VERSION: &str = "1.8.25";

/// Ensures the requested R toolchain is installed system-wide.
pub fn install_r(shell: &Shell, version: &ResolvedRVersion, progress: &Progress) -> Result<()> {
    let check_task = progress.task(format!(
        "Checking existing R {} installation",
        version.version
    ));
    let r_already_installed = is_r_already_installed(shell, version)?;
    if r_already_installed {
        check_task.finish_with_message(format!("Using existing R {}", version.version));
    } else {
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
    }

    ensure_quarto(shell, progress).context("failed to provision Quarto")?;
    ensure_pandoc(shell, progress).context("failed to provision pandoc")?;
    ensure_tinytex(shell, progress).context("failed to provision TinyTeX")?;

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
        "Installing pak system requirements",
        "pak system requirements installed",
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

fn ensure_quarto(shell: &Shell, progress: &Progress) -> Result<()> {
    ensure_curl(shell, progress)?;

    let check_task = progress.task(format!("Checking existing Quarto {QUARTO_VERSION}"));
    let already_installed = match cmd!(shell, "quarto --version")
        .quiet()
        .ignore_status()
        .read()
    {
        Ok(output) => output.contains(QUARTO_VERSION),
        Err(_) => false,
    };

    if already_installed {
        check_task.finish_with_message(format!("Using existing Quarto {QUARTO_VERSION}"));
        return Ok(());
    }
    check_task.finish_with_message(format!("Quarto {QUARTO_VERSION} not detected; installing"));

    run_command(
        progress,
        format!("Creating /opt/quarto/{QUARTO_VERSION}"),
        format!("Prepared /opt/quarto/{QUARTO_VERSION}"),
        cmd!(shell, "sudo mkdir -p /opt/quarto/{QUARTO_VERSION}"),
    )?;

    let tarball_path = format!("/tmp/quarto-{QUARTO_VERSION}.tar.gz");
    let download_url = format!(
        "https://github.com/quarto-dev/quarto-cli/releases/download/v{}/quarto-{}-linux-amd64.tar.gz",
        QUARTO_VERSION, QUARTO_VERSION
    );

    run_command(
        progress,
        format!("Downloading Quarto {QUARTO_VERSION} bundle"),
        format!("Downloaded Quarto {QUARTO_VERSION} bundle"),
        cmd!(shell, "curl -fsSL -o {tarball_path} -L {download_url}"),
    )?;

    run_command(
        progress,
        format!("Extracting Quarto {QUARTO_VERSION} bundle"),
        format!("Installed Quarto {QUARTO_VERSION} to /opt/quarto/{QUARTO_VERSION}"),
        cmd!(
            shell,
            "sudo tar -xzf {tarball_path} -C /opt/quarto/{QUARTO_VERSION} --strip-components=1"
        ),
    )?;

    run_command(
        progress,
        "Cleaning temporary Quarto archive",
        "Removed temporary Quarto archive",
        cmd!(shell, "rm -f {tarball_path}"),
    )?;

    run_command(
        progress,
        "Linking Quarto binary",
        format!("Linked /usr/local/bin/quarto -> /opt/quarto/{QUARTO_VERSION}/bin/quarto"),
        cmd!(
            shell,
            "sudo ln -sf /opt/quarto/{QUARTO_VERSION}/bin/quarto /usr/local/bin/quarto"
        ),
    )?;

    progress.println(format!("Quarto {QUARTO_VERSION} installation completed"));

    Ok(())
}

fn ensure_pandoc(shell: &Shell, progress: &Progress) -> Result<()> {
    let check_task = progress.task("Checking existing pandoc");
    let already_installed = cmd!(shell, "pandoc --version")
        .quiet()
        .ignore_status()
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if already_installed {
        check_task.finish_with_message("Using existing pandoc");
        return Ok(());
    }
    check_task.finish_with_message("pandoc not detected; installing");

    run_command(
        progress,
        "Updating apt metadata for pandoc",
        "apt metadata updated for pandoc",
        cmd!(
            shell,
            "sudo env DEBIAN_FRONTEND=noninteractive apt-get update -y -qq"
        ),
    )?;

    run_command(
        progress,
        "Installing pandoc",
        "pandoc installed",
        cmd!(
            shell,
            "sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y pandoc"
        ),
    )?;

    progress.println("pandoc installation completed");

    Ok(())
}

fn ensure_tinytex(shell: &Shell, progress: &Progress) -> Result<()> {
    let check_task = progress.task("Checking existing TinyTeX");
    if tinytex_is_installed(shell) {
        check_task.finish_with_message("Using existing TinyTeX");
        return Ok(());
    }
    check_task.finish_with_message("TinyTeX not detected; installing");

    run_command(
        progress,
        "Installing TinyTeX via Quarto",
        "TinyTeX installed via Quarto",
        cmd!(
            shell,
            "quarto install tinytex --no-prompt --log-level warning"
        ),
    )?;

    if !tinytex_is_installed(shell) {
        bail!("TinyTeX installation via Quarto did not succeed");
    }

    link_tinytex_binaries(shell, progress)?;

    progress.println("TinyTeX installation completed");

    Ok(())
}

fn tinytex_is_installed(shell: &Shell) -> bool {
    let cli_available = cmd!(shell, "tlmgr --version")
        .quiet()
        .ignore_status()
        .run()
        .is_ok();
    if cli_available {
        return true;
    }

    if let Ok(output) = cmd!(shell, "quarto list tools").ignore_status().read() {
        for line in output.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("tinytex") {
                return !trimmed.contains("Not installed");
            }
        }
    }

    false
}

fn link_tinytex_binaries(shell: &Shell, progress: &Progress) -> Result<()> {
    let mut bin_dirs = Vec::new();

    if let Ok(path_output) = cmd!(shell, "command -v tlmgr")
        .quiet()
        .ignore_status()
        .read()
    {
        let path = Path::new(path_output.trim());
        if let Some(parent) = path.parent() {
            bin_dirs.push(parent.to_path_buf());
        }
    }

    if let Some(home_dir) = env::var_os("HOME").map(PathBuf::from) {
        let bin_root = home_dir.join(".TinyTeX").join("bin");
        if let Ok(entries) = std::fs::read_dir(&bin_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && !bin_dirs.iter().any(|existing| existing == &path) {
                    bin_dirs.push(path);
                }
            }
        }
    }

    if bin_dirs.is_empty() {
        progress.println("TinyTeX binaries not located; skipping symlink creation");
        return Ok(());
    }

    for binary in ["tlmgr", "pdflatex", "xelatex", "lualatex"] {
        if let Some(source) = bin_dirs
            .iter()
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.exists())
        {
            run_command(
                progress,
                format!("Linking TinyTeX {binary}"),
                format!("Linked /usr/local/bin/{binary}"),
                cmd!(shell, "sudo ln -sf {source} /usr/local/bin/{binary}"),
            )?;
        }
    }

    Ok(())
}

fn ensure_curl(shell: &Shell, progress: &Progress) -> Result<()> {
    if cmd!(shell, "curl --version")
        .quiet()
        .ignore_status()
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
    {
        return Ok(());
    }

    run_command(
        progress,
        "Updating apt metadata for curl",
        "apt metadata updated for curl",
        cmd!(
            shell,
            "sudo env DEBIAN_FRONTEND=noninteractive apt-get update -y -qq"
        ),
    )?;

    run_command(
        progress,
        "Installing curl",
        "curl installed",
        cmd!(
            shell,
            "sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y curl"
        ),
    )
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
