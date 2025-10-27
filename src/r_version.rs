use std::{collections::HashMap, env, fs};

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;

const API_ENDPOINT: &str = "https://api.r-hub.io/rversions/resolve";

/// Metadata describing a resolved R toolchain download.
#[derive(Debug, Clone, Deserialize)]
pub struct ResolvedRVersion {
    /// Human readable version string (e.g. `4.3.3`).
    pub version: String,
    /// Download URL for the platform-specific installer.
    pub url: String,
    /// Build type, used to detect special channels like `next` or `devel`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

impl ResolvedRVersion {
    /// Returns the directory name used under `/opt/R/` by the upstream installer.
    pub fn install_dir_name(&self) -> &str {
        match self.kind.as_deref() {
            Some(kind @ ("next" | "devel")) => kind,
            _ => self.version.as_str(),
        }
    }
}

/// Resolves the user provided version specifier to a concrete installer download.
pub fn resolve(spec: &str) -> Result<ResolvedRVersion> {
    let normalized = normalize_spec(spec);
    let platform = linux_platform().context("failed to determine Linux distribution")?;
    let mut url = format!("{API_ENDPOINT}/{normalized}/{platform}");

    if let Some(arch) = detect_arch() {
        url.push('/');
        url.push_str(arch);
    }

    let client = http_client()?;
    let response = client
        .get(url.clone())
        .send()
        .with_context(|| format!("failed to contact version API at {url}"))?
        .error_for_status()
        .with_context(|| format!("version API returned error for request {url}"))?;

    response
        .json::<ResolvedRVersion>()
        .with_context(|| format!("failed to decode version metadata from {url}"))
}

fn http_client() -> Result<Client> {
    Client::builder()
        .user_agent(format!("revdeprun/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to create HTTP client")
}

/// Normalises the version specification following the behaviour of setup-r.
pub fn normalize_spec(spec: &str) -> String {
    match spec.trim() {
        "latest" | "4" | "4.x" | "4.x.x" => "release".to_string(),
        "3" | "3.x" | "3.x.x" => "3.6.3".to_string(),
        value if value.ends_with(".x") => value.trim_end_matches(".x").to_string(),
        value if value.starts_with("oldrel-") => value.replacen("oldrel-", "oldrel/", 1),
        value => value.to_string(),
    }
}

fn detect_arch() -> Option<&'static str> {
    match env::consts::ARCH {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("arm64"),
        other => {
            eprintln!(
                "Warning: unsupported architecture '{other}', falling back to default download."
            );
            None
        }
    }
}

fn linux_platform() -> Result<String> {
    if let Ok(override_value) = env::var("REVDEPRUN_LINUX_PLATFORM") {
        if !override_value.trim().is_empty() {
            return Ok(override_value);
        }
    }

    let os_release =
        fs::read_to_string("/etc/os-release").context("failed to read /etc/os-release")?;
    let pairs = parse_os_release(&os_release);
    let id = pairs
        .get("ID")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing ID in /etc/os-release"))?;
    let version = pairs
        .get("VERSION_ID")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing VERSION_ID in /etc/os-release"))?;

    Ok(format!("linux-{id}-{version}"))
}

fn parse_os_release(contents: &str) -> HashMap<String, String> {
    contents
        .lines()
        .filter_map(|line| {
            if line.trim_start().starts_with('#') || !line.contains('=') {
                return None;
            }
            let (key, value) = line.split_once('=').unwrap();
            let key = key.trim().to_string();
            let value = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            Some((key, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_version_spec() {
        assert_eq!(normalize_spec("latest"), "release");
        assert_eq!(normalize_spec("4.x"), "release");
        assert_eq!(normalize_spec("3.x"), "3.6.3");
        assert_eq!(normalize_spec("4.2.x"), "4.2");
        assert_eq!(normalize_spec("oldrel-1"), "oldrel/1");
        assert_eq!(normalize_spec(" 4.3.2 "), "4.3.2");
    }

    #[test]
    fn parses_os_release() {
        let sample = r#"NAME="Ubuntu"
VERSION="22.04.4 LTS (Jammy Jellyfish)"
ID=ubuntu
ID_LIKE=debian
VERSION_ID="22.04"
PRETTY_NAME="Ubuntu 22.04.4 LTS"
VERSION_CODENAME=jammy
UBUNTU_CODENAME=jammy
"#;

        let pairs = parse_os_release(sample);
        assert_eq!(pairs.get("ID").map(String::as_str), Some("ubuntu"));
        assert_eq!(pairs.get("VERSION_ID").map(String::as_str), Some("22.04"));
    }
}
