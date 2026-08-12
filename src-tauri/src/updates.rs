use semver::Version;
use serde::{Deserialize, Serialize};
use std::{ffi::c_void, mem::size_of, ptr, time::Instant};
use windows::{
    core::{w, Error as WindowsError, PCWSTR},
    Win32::{
        Networking::WinHttp::{
            WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
            WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
            WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
        },
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
};

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

struct WinHttpHandle(*mut c_void);

impl WinHttpHandle {
    fn from_raw(handle: *mut c_void, operation: &str) -> Result<Self, String> {
        if handle.is_null() {
            Err(format!(
                "{operation} failed: {}",
                WindowsError::from_win32()
            ))
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for WinHttpHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a non-null HINTERNET returned by WinHTTP and this
        // RAII owner closes it exactly once after all dependent handles are gone.
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
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
    let host = wide_null(UPDATE_ENDPOINT_HOST);
    let path = wide_null(UPDATE_ENDPOINT_PATH);

    // SAFETY: compile-time and owned UTF-16 strings remain valid for each
    // synchronous call. Returned handles are checked and immediately owned by
    // `WinHttpHandle`, which closes them in reverse dependency order.
    let session = unsafe {
        WinHttpHandle::from_raw(
            WinHttpOpen(
                w!("HD2-Macro-Terminal/2.0"),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "WinHttpOpen",
        )?
    };

    // SAFETY: `session` is a live WinHTTP session and the timeout values are
    // finite milliseconds. WinHTTP does not retain references to Rust memory.
    unsafe { WinHttpSetTimeouts(session.raw(), 1_500, 2_500, 2_500, 4_000) }
        .map_err(|error| format!("WinHttpSetTimeouts failed: {error}"))?;

    // SAFETY: `session` remains live, and `host` is a nul-terminated buffer
    // that outlives the synchronous connection creation call.
    let connection = unsafe {
        WinHttpHandle::from_raw(
            WinHttpConnect(
                session.raw(),
                PCWSTR(host.as_ptr()),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ),
            "WinHttpConnect",
        )?
    };

    // SAFETY: `connection` remains live, `path` is nul-terminated, and all
    // optional pointer arguments are intentionally null. The request is HTTPS.
    let request = unsafe {
        WinHttpHandle::from_raw(
            WinHttpOpenRequest(
                connection.raw(),
                w!("GET"),
                PCWSTR(path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "WinHttpOpenRequest",
        )?
    };

    let headers: Vec<u16> = "Accept: application/vnd.github+json\r\n"
        .encode_utf16()
        .collect();
    // SAFETY: `request` is live, the header slice is valid for the synchronous
    // call, and there is no request body or asynchronous callback context.
    unsafe { WinHttpSendRequest(request.raw(), Some(&headers), None, 0, 0, 0) }
        .map_err(|error| format!("WinHttpSendRequest failed: {error}"))?;
    // SAFETY: `request` is live and the reserved pointer must be null.
    unsafe { WinHttpReceiveResponse(request.raw(), ptr::null_mut()) }
        .map_err(|error| format!("WinHttpReceiveResponse failed: {error}"))?;

    let mut status_code = 0_u32;
    let mut status_length = size_of::<u32>() as u32;
    let mut header_index = 0_u32;
    // SAFETY: the status and length pointers are valid writable storage for the
    // synchronous query, and numeric status-code mode writes exactly a u32.
    unsafe {
        WinHttpQueryHeaders(
            request.raw(),
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status_code as *mut u32).cast()),
            &mut status_length,
            &mut header_index,
        )
    }
    .map_err(|error| format!("WinHttpQueryHeaders failed: {error}"))?;
    if status_code != 200 {
        return Err(format!(
            "Update endpoint returned unexpected HTTP status {status_code}"
        ));
    }

    let mut body = Vec::with_capacity(8 * 1024);
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let mut bytes_read = 0_u32;
        // SAFETY: `request` is live and `buffer` is valid writable storage for
        // its declared length. WinHTTP writes the actual count to `bytes_read`.
        unsafe {
            WinHttpReadData(
                request.raw(),
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut bytes_read,
            )
        }
        .map_err(|error| format!("WinHttpReadData failed: {error}"))?;
        if bytes_read == 0 {
            break;
        }
        let bytes_read = bytes_read as usize;
        if body.len() + bytes_read > MAX_RESPONSE_BYTES {
            return Err("Update endpoint response is too large".to_owned());
        }
        body.extend_from_slice(&buffer[..bytes_read]);
    }

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
