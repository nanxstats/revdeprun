use std::{fs, io::Write, path::Path};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Deserializer};
use tempfile::NamedTempFile;
use xshell::{Shell, cmd};

use crate::{
    progress::Progress,
    r_scripts::{self, RConfig},
    util,
    workspace::Workspace,
};

#[derive(Debug, Deserialize)]
struct SysreqsPayload {
    #[serde(default, deserialize_with = "string_or_vec")]
    install_scripts: Vec<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    post_install: Vec<String>,
}

fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error as _;
    use serde_json::Value;

    match Value::deserialize(deserializer)? {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => Ok(vec![s]),
        Value::Array(items) => items
            .into_iter()
            .map(|value| match value {
                Value::String(s) => Ok(s),
                other => Err(D::Error::custom(format!(
                    "expected string in array, got {other}"
                ))),
            })
            .collect(),
        other => Err(D::Error::custom(format!(
            "expected string, array, or null, got {other}"
        ))),
    }
}

/// Resolves and installs system requirements for reverse dependencies.
pub fn install_reverse_dep_sysreqs(
    shell: &Shell,
    workspace: &Workspace,
    repo_path: &Path,
    num_workers: usize,
    progress: &Progress,
) -> Result<()> {
    let max_connections = util::optimal_max_connections(num_workers);
    let package_name = read_package_name(repo_path)?;
    let script_contents = build_sysreqs_script(&package_name, num_workers);
    let mut script = NamedTempFile::new_in(workspace.temp_dir())
        .context("failed to create temporary sysreqs R script")?;
    script
        .write_all(script_contents.as_bytes())
        .context("failed to write sysreqs R script")?;

    let script_path = script.path().to_owned();
    let _dir_guard = shell.push_dir(repo_path);

    let task = progress.task(format!(
        "Resolving system requirements for reverse dependencies of {package_name}"
    ));
    let max_connections_arg = max_connections.to_string();
    let output = cmd!(
        shell,
        "Rscript --vanilla --max-connections={max_connections_arg} {script_path}"
    )
    .quiet()
    .ignore_status()
    .output();

    let output = match output {
        Ok(output) if output.status.success() => {
            task.finish_with_message(format!("System requirements resolved for {package_name}"));
            output
        }
        Ok(output) => {
            task.fail(format!(
                "Failed to resolve system requirements for {package_name}"
            ));
            util::emit_command_output(
                progress,
                "reverse dependency sysreq resolution",
                &output.stdout,
                &output.stderr,
            );
            bail!(
                "sysreq resolution script failed with status {}",
                output.status
            );
        }
        Err(err) => {
            task.fail(format!(
                "Launching sysreq resolution for {package_name} failed"
            ));
            return Err(err).context("failed to resolve reverse dependency sysreqs");
        }
    };

    let stdout =
        String::from_utf8(output.stdout).context("sysreq resolution emitted non-UTF-8 output")?;
    let payload: SysreqsPayload =
        serde_json::from_str(stdout.trim()).context("failed to parse sysreq resolution output")?;

    install_scripts(shell, &package_name, &payload.install_scripts, progress)?;
    run_post_install(shell, &package_name, &payload.post_install, progress)?;

    Ok(())
}

fn install_scripts(
    shell: &Shell,
    package_name: &str,
    install_scripts: &[String],
    progress: &Progress,
) -> Result<()> {
    if install_scripts.is_empty() {
        progress.println(format!(
            "No additional dependencies required for checking reverse dependencies of {package_name}."
        ));
        return Ok(());
    }

    progress.println(format!(
        "Installing packages required for checking reverse dependencies of {package_name}..."
    ));
    for script in install_scripts {
        let label = format!("sudo sh -c {}", script);
        let task = progress.task(format!("Running {label}"));
        let output = cmd!(shell, "sudo sh -c {script}")
            .quiet()
            .ignore_status()
            .output();

        match output {
            Ok(output) if output.status.success() => {
                task.finish_with_message(format!("{label} succeeded"));
            }
            Ok(output) => {
                task.fail(format!("{label} failed"));
                util::emit_command_output(progress, &label, &output.stdout, &output.stderr);
                bail!("revdep dependency package installation failed: {}", label);
            }
            Err(err) => {
                task.fail(format!("{label} failed to start"));
                return Err(err).context("failed to execute revdep dependency installation");
            }
        }
    }

    Ok(())
}

fn run_post_install(
    shell: &Shell,
    package_name: &str,
    post_install: &[String],
    progress: &Progress,
) -> Result<()> {
    if post_install.is_empty() {
        return Ok(());
    }

    progress.println(format!(
        "Running post-install hooks for reverse dependencies of {package_name}..."
    ));
    for command in post_install {
        let label = format!("sudo sh -c {}", command);
        let task = progress.task(format!("Running {label}"));
        let output = cmd!(shell, "sudo sh -c {command}")
            .quiet()
            .ignore_status()
            .output();

        match output {
            Ok(output) if output.status.success() => {
                task.finish_with_message(format!("{label} succeeded"));
            }
            Ok(output) => {
                task.fail(format!("{label} failed"));
                util::emit_command_output(progress, &label, &output.stdout, &output.stderr);
                bail!("post-install command failed: {}", label);
            }
            Err(err) => {
                task.fail(format!("{label} failed to start"));
                return Err(err).context("failed to execute post-install command");
            }
        }
    }

    Ok(())
}

fn read_package_name(repo_path: &Path) -> Result<String> {
    let description_path = repo_path.join("DESCRIPTION");
    let contents = fs::read_to_string(&description_path).with_context(|| {
        format!(
            "failed to read package DESCRIPTION at {}",
            description_path.display()
        )
    })?;

    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("Package:") {
            let name = rest.trim();
            if name.is_empty() {
                bail!("package DESCRIPTION has empty Package field");
            }
            return Ok(name.to_string());
        }
    }

    Err(anyhow!(
        "could not find Package field in {}",
        description_path.display()
    ))
}

/// Assembles the system requirement resolution script from
/// `assets/r/sysreqs.R`.
fn build_sysreqs_script(package_name: &str, num_workers: usize) -> String {
    let config = RConfig::new()
        .string("pkg_name", package_name)
        .integer("install_workers", num_workers.max(1));

    r_scripts::assemble(&config, &[r_scripts::SYSREQS])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn reads_package_name_from_description() {
        let dir = tempdir().expect("tempdir");
        let description_path = dir.path().join("DESCRIPTION");
        let mut file = File::create(&description_path).expect("create DESCRIPTION");
        writeln!(file, "Package: example").expect("write package");
        let name = read_package_name(dir.path()).expect("package name");
        assert_eq!(name, "example");
    }

    #[test]
    fn build_script_contains_expected_fragments() {
        let script = build_sysreqs_script("ggsci", 4);
        assert!(script.starts_with("# Configuration generated by revdeprun ----\n"));
        assert!(script.contains("pkg_name <- 'ggsci'"));
        assert!(script.contains("install_workers <- 4"));
        assert!(script.contains("Ncpus = install_workers"));
        assert!(script.contains("options(warn = 2)"));
        assert!(script.contains("packages = pkg_name,"));
        assert!(script.contains("tools::package_dependencies"));
        assert!(script.contains("pak::pkg_sysreqs"));
        assert!(script.contains("ensure_installed(\"pak\")"));
        assert!(script.contains("available.packages"));
        assert!(script.contains("jsonlite::toJSON"));
        assert!(script.contains("Sys.setenv(NOT_CRAN = \"true\")"));
        assert!(script.contains("setdiff(revdeps, base_pkgs)"));
        assert!(script.contains("sysreqs_platform = \"ubuntu\""));
    }

    #[test]
    fn build_script_defines_configuration_before_use() {
        let script = build_sysreqs_script("ggsci", 4);
        let defined = script
            .find("pkg_name <- 'ggsci'")
            .expect("pkg_name assignment");
        let used = script.find("packages = pkg_name,").expect("pkg_name use");
        assert!(defined < used);
        let defined = script
            .find("install_workers <- 4")
            .expect("workers assignment");
        let used = script.find("Ncpus = install_workers").expect("workers use");
        assert!(defined < used);
    }

    #[test]
    fn build_script_escapes_package_name() {
        let script = build_sysreqs_script("it's", 1);
        assert!(script.contains("pkg_name <- 'it\\'s'"));
        assert!(script.contains("install_workers <- 1\n"));
    }

    #[test]
    fn deserializes_string_install_script() {
        let json = r#"
            {
                "install_scripts": "apt-get install libcurl4",
                "post_install": []
            }
        "#;
        let payload: SysreqsPayload =
            serde_json::from_str(json).expect("string payload should deserialize");
        assert_eq!(
            payload.install_scripts,
            vec!["apt-get install libcurl4".to_string()]
        );
        assert!(payload.post_install.is_empty());
    }

    #[test]
    fn deserializes_null_install_scripts() {
        let json = r#"
            {
                "install_scripts": null,
                "post_install": "echo done"
            }
        "#;
        let payload: SysreqsPayload =
            serde_json::from_str(json).expect("null payload should deserialize");
        assert!(payload.install_scripts.is_empty());
        assert_eq!(payload.post_install, vec!["echo done".to_string()]);
    }
}
