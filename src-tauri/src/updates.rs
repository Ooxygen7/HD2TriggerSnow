use semver::Version;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use windows::{
    core::{w, PCWSTR},
    Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};

use crate::network;

const UPDATE_ENDPOINT_HOST: &str = "update.unsnow.online";
const UPDATE_ENDPOINT_PATH: &str = "/api/v1/releases/latest";
const RELEASES_PAGE_URL: &str =
    "https://github.com/Ooxygen7/HELLDIVERS2_QuickStratagemTool/releases/latest";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub version: String,
    pub release_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEndpointDiagnostics {
    pub endpoint_host: &'static str,
    pub reachable: bool,
    pub valid_response: bool,
    pub latest_version: Option<String>,
    pub update_available: Option<bool>,
    pub latency_ms: u128,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReleasePayload {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

pub fn check_for_update() -> Result<Option<UpdateInfo>, String> {
    let payload = fetch_latest_release()?;
    available_update(&payload, env!("CARGO_PKG_VERSION"))
}

pub fn diagnose_endpoint() -> UpdateEndpointDiagnostics {
    let started = Instant::now();
    match fetch_latest_release() {
        Ok(payload) => match parse_release_tag(&payload.tag_name) {
            Ok(version) => {
                let update_available = available_update(&payload, env!("CARGO_PKG_VERSION"))
                    .ok()
                    .map(|update| update.is_some());
                UpdateEndpointDiagnostics {
                    endpoint_host: UPDATE_ENDPOINT_HOST,
                    reachable: true,
                    valid_response: true,
                    latest_version: Some(version.to_string()),
                    update_available,
                    latency_ms: started.elapsed().as_millis(),
                    error: None,
                }
            }
            Err(error) => UpdateEndpointDiagnostics {
                endpoint_host: UPDATE_ENDPOINT_HOST,
                reachable: true,
                valid_response: false,
                latest_version: None,
                update_available: None,
                latency_ms: started.elapsed().as_millis(),
                error: Some(compact_error(&error)),
            },
        },
        Err(error) => UpdateEndpointDiagnostics {
            endpoint_host: UPDATE_ENDPOINT_HOST,
            reachable: false,
            valid_response: false,
            latest_version: None,
            update_available: None,
            latency_ms: started.elapsed().as_millis(),
            error: Some(compact_error(&error)),
        },
    }
}

pub fn open_releases_page() -> Result<(), String> {
    let url = wide_null(RELEASES_PAGE_URL);
    // SAFETY: all strings are valid, nul-terminated UTF-16 buffers that remain
    // alive for the synchronous ShellExecuteW call. No window handle is needed.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    let result_code = result.0 as usize;
    if result_code <= 32 {
        Err(format!(
            "Windows could not open the GitHub Releases page (code {result_code})"
        ))
    } else {
        Ok(())
    }
}

fn fetch_latest_release() -> Result<ReleasePayload, String> {
    let body = network::fetch_https(
        UPDATE_ENDPOINT_HOST,
        UPDATE_ENDPOINT_PATH,
        "application/vnd.github+json",
        MAX_RESPONSE_BYTES,
    )?;
    serde_json::from_slice(&body).map_err(|error| format!("Invalid update response: {error}"))
}

fn available_update(payload: &ReleasePayload, current: &str) -> Result<Option<UpdateInfo>, String> {
    if payload.draft || payload.prerelease {
        return Ok(None);
    }
    let current_version = Version::parse(current)
        .map_err(|error| format!("Invalid application version {current:?}: {error}"))?;
    let latest_version = parse_release_tag(&payload.tag_name)?;
    if latest_version <= current_version {
        return Ok(None);
    }

    let release_name = payload
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| name.chars().take(160).collect());
    Ok(Some(UpdateInfo {
        current_version: current_version.to_string(),
        version: latest_version.to_string(),
        release_name,
    }))
}

fn parse_release_tag(tag: &str) -> Result<Version, String> {
    let tag = tag.trim();
    let tag = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag);
    let tag = tag.strip_prefix('.').unwrap_or(tag);
    Version::parse(tag).map_err(|error| format!("Invalid GitHub Release tag {tag:?}: {error}"))
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn compact_error(error: &str) -> String {
    error
        .replace(['\r', '\n', '\t'], " ")
        .chars()
        .take(320)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(tag_name: &str) -> ReleasePayload {
        ReleasePayload {
            tag_name: tag_name.to_owned(),
            name: Some("Stable release".to_owned()),
            draft: false,
            prerelease: false,
        }
    }

    #[test]
    fn accepts_both_repository_tag_styles() {
        assert_eq!(parse_release_tag("v.2.0.2").unwrap(), Version::new(2, 0, 2));
        assert_eq!(parse_release_tag("v2.1.0").unwrap(), Version::new(2, 1, 0));
    }

    #[test]
    fn reports_only_strictly_newer_releases() {
        let update = available_update(&payload("v.2.0.2"), "2.0.1")
            .unwrap()
            .expect("a newer release should be reported");
        assert_eq!(update.current_version, "2.0.1");
        assert_eq!(update.version, "2.0.2");
        assert!(available_update(&payload("v.2.0.1"), "2.0.1")
            .unwrap()
            .is_none());
        assert!(available_update(&payload("v.1.9.9"), "2.0.1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn ignores_drafts_and_prereleases_even_if_the_version_is_newer() {
        let mut draft = payload("v.3.0.0");
        draft.draft = true;
        assert!(available_update(&draft, "2.0.1").unwrap().is_none());

        let mut prerelease = payload("v.3.0.0-beta.1");
        prerelease.prerelease = true;
        assert!(available_update(&prerelease, "2.0.1").unwrap().is_none());
    }

    #[test]
    fn endpoint_diagnostics_error_text_is_single_line_and_bounded() {
        let error = format!("first\r\nsecond\t{}", "x".repeat(500));
        let compact = compact_error(&error);
        assert!(!compact.contains(['\r', '\n', '\t']));
        assert!(compact.chars().count() <= 320);
    }

    #[test]
    fn rejects_non_semver_release_tags() {
        assert!(available_update(&payload("latest"), "2.0.1").is_err());
    }

    #[test]
    #[ignore = "requires the public update.unsnow.online endpoint"]
    fn public_endpoint_returns_a_valid_github_release() {
        let release = fetch_latest_release().expect("the public update endpoint should respond");
        assert!(!release.draft);
        assert!(!release.prerelease);
        parse_release_tag(&release.tag_name).expect("the latest Release tag should be SemVer");
        assert!(
            available_update(&release, env!("CARGO_PKG_VERSION"))
                .expect("the live release should be comparable")
                .is_none(),
            "the build version must not lag behind the currently published Release"
        );
    }
}
