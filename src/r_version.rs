use std::{collections::HashMap, env, fs};

use anyhow::{Context, Result, bail};
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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ApiResponse {
    Resolved(ResolvedRVersion),
    Error(ApiError),
}

#[derive(Debug, Deserialize)]
struct ApiError {
    error: String,
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
    let client = http_client()?;

    resolve_for_platform(&normalized, &platform, detect_arch(), |url| {
        request_version(&client, url)
    })
}

fn resolve_for_platform<F>(
    normalized: &str,
    initial_platform: &str,
    arch: Option<&str>,
    mut request: F,
) -> Result<ResolvedRVersion>
where
    F: FnMut(&str) -> Result<ApiResponse>,
{
    let mut platform = initial_platform.to_string();

    loop {
        let url = version_url(normalized, &platform, arch);
        match request(&url)? {
            ApiResponse::Resolved(version) => return Ok(version),
            ApiResponse::Error(error) => {
                if is_unsupported_linux_distro(&error.error) {
                    if let Some(fallback) = previous_debian_platform(&platform) {
                        eprintln!(
                            "Warning: R Hub does not support {platform}; retrying with {fallback}."
                        );
                        platform = fallback;
                        continue;
                    }
                }

                bail!(
                    "version API returned error for request {url}: {}",
                    error.error
                );
            }
        }
    }
}

fn request_version(client: &Client, url: &str) -> Result<ApiResponse> {
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("failed to contact version API at {url}"))?
        .json::<ApiResponse>()
        .with_context(|| format!("failed to decode version API response from {url}"))?;

    Ok(response)
}

fn version_url(normalized: &str, platform: &str, arch: Option<&str>) -> String {
    let mut url = format!("{API_ENDPOINT}/{normalized}/{platform}");
    if let Some(arch) = arch {
        url.push('/');
        url.push_str(arch);
    }
    url
}

fn is_unsupported_linux_distro(error: &str) -> bool {
    error.contains("Unknown Linux distro")
}

fn previous_debian_platform(platform: &str) -> Option<String> {
    let version = platform
        .strip_prefix("linux-debian-")?
        .parse::<u32>()
        .ok()?;
    let previous = version.checked_sub(1)?;
    (previous > 0).then(|| format!("linux-debian-{previous}"))
}

fn http_client() -> Result<Client> {
    Client::builder()
        .user_agent(format!("revdeprun/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to create HTTP client")
}

/// Normalizes the version specification following the behavior of setup-r.
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
    fn normalizes_version_spec() {
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
VERSION="26.04 LTS (Resolute Raccoon)"
ID=ubuntu
ID_LIKE=debian
VERSION_ID="26.04"
PRETTY_NAME="Ubuntu 26.04 LTS"
VERSION_CODENAME=resolute
UBUNTU_CODENAME=resolute
"#;

        let pairs = parse_os_release(sample);
        assert_eq!(pairs.get("ID").map(String::as_str), Some("ubuntu"));
        assert_eq!(pairs.get("VERSION_ID").map(String::as_str), Some("26.04"));
    }

    #[test]
    fn builds_ubuntu_26_04_version_url() {
        assert_eq!(
            version_url("devel", "linux-ubuntu-26.04", Some("x86_64")),
            format!("{API_ENDPOINT}/devel/linux-ubuntu-26.04/x86_64")
        );
    }

    #[test]
    fn falls_back_to_the_next_older_debian_platform() {
        let mut requests = Vec::new();
        let resolved = resolve_for_platform("release", "linux-debian-14", Some("x86_64"), |url| {
            requests.push(url.to_string());
            if url.contains("linux-debian-14") {
                Ok(ApiResponse::Error(ApiError {
                    error: "Error: Unknown Linux distro: 'linux-debian-14'.".to_string(),
                }))
            } else {
                Ok(ApiResponse::Resolved(ResolvedRVersion {
                    version: "4.6.1".to_string(),
                    url: "https://example.com/r.deb".to_string(),
                    kind: Some("release".to_string()),
                }))
            }
        })
        .unwrap();

        assert_eq!(resolved.version, "4.6.1");
        assert_eq!(
            requests,
            [
                format!("{API_ENDPOINT}/release/linux-debian-14/x86_64"),
                format!("{API_ENDPOINT}/release/linux-debian-13/x86_64"),
            ]
        );
    }

    #[test]
    fn does_not_fall_back_for_invalid_r_version() {
        let mut request_count = 0;
        let error =
            resolve_for_platform("not-a-version", "linux-debian-14", Some("x86_64"), |_| {
                request_count += 1;
                Ok(ApiResponse::Error(ApiError {
                    error: "Error: Invalid version specification: 'not-a-version'.".to_string(),
                }))
            })
            .unwrap_err();

        assert_eq!(request_count, 1);
        assert!(error.to_string().contains("Invalid version specification"));
    }

    #[test]
    fn only_falls_back_for_numeric_debian_platforms() {
        assert_eq!(
            previous_debian_platform("linux-debian-14"),
            Some("linux-debian-13".to_string())
        );
        assert_eq!(previous_debian_platform("linux-debian-testing"), None);
        assert_eq!(previous_debian_platform("linux-ubuntu-26.04"), None);
    }

    #[test]
    fn parses_supported_and_unsupported_api_responses() {
        let supported = r#"{
            "version": "4.6.1",
            "type": "release",
            "url": "https://cdn.posit.co/r/debian-13/pkgs/r-4.6.1_1_amd64.deb"
        }"#;
        let unsupported = r#"{
            "version": "release",
            "os": "linux-debian-14",
            "arch": "x86_64",
            "error": "Error: Unknown Linux distro: 'linux-debian-14'."
        }"#;

        assert!(matches!(
            serde_json::from_str::<ApiResponse>(supported).unwrap(),
            ApiResponse::Resolved(_)
        ));
        assert!(matches!(
            serde_json::from_str::<ApiResponse>(unsupported).unwrap(),
            ApiResponse::Error(_)
        ));
    }

    #[test]
    fn parses_ubuntu_26_04_api_response() {
        let response = r#"{
            "ppm-binaries": true,
            "ppm-binary-url": "resolute",
            "version": "4.7.0",
            "nickname": "Unsuffered Consequences",
            "type": "devel",
            "url": "https://cdn.posit.co/r/ubuntu-2604/pkgs/r-devel_1_amd64.deb",
            "date": null
        }"#;

        let parsed = serde_json::from_str::<ApiResponse>(response).unwrap();
        let ApiResponse::Resolved(resolved) = parsed else {
            panic!("Ubuntu 26.04 response should resolve successfully");
        };
        assert_eq!(resolved.version, "4.7.0");
        assert_eq!(resolved.kind.as_deref(), Some("devel"));
        assert_eq!(
            resolved.url,
            "https://cdn.posit.co/r/ubuntu-2604/pkgs/r-devel_1_amd64.deb"
        );
    }
}
