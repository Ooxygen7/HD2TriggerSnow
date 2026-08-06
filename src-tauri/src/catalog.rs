//! Online built-in stratagem catalog refresh.
//!
//! The upstream project embeds `defaultStratagemDB` in `ui/index.html`. There is
//! no separate JSON feed, so this module downloads that page from GitHub and
//! extracts a validated array for the renderer.

use serde_json::{Map, Value};
use std::{ffi::c_void, mem::size_of, ptr};
use windows::{
    core::{w, Error as WindowsError, PCWSTR},
    Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
        WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    },
};

const CATALOG_HOST: &str = "raw.githubusercontent.com";
const CATALOG_PATH: &str = "/Ooxygen7/HELLDIVERS2_QuickStratagemTool/main/ui/index.html";
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_STRATAGEMS: usize = 512;
const MAX_ID_LEN: usize = 100;
const MAX_SEQ_LEN: usize = 32;

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

/// Download and parse the built-in stratagem list published on GitHub `main`.
pub fn fetch_remote_builtin_strats() -> Result<Value, String> {
    let html = fetch_catalog_html()?;
    let array_js = extract_default_stratagem_db(&html)?;
    let json_text = js_array_literal_to_json(&array_js)?;
    let value: Value = serde_json::from_str(&json_text)
        .map_err(|error| format!("Cannot parse remote stratagem catalog: {error}"))?;
    validate_stratagem_catalog(&value)?;
    Ok(value)
}

fn fetch_catalog_html() -> Result<String, String> {
    let body = https_get(
        CATALOG_HOST,
        CATALOG_PATH,
        "Accept: text/plain,text/html,*/*\r\n",
        MAX_RESPONSE_BYTES,
    )?;
    String::from_utf8(body).map_err(|error| format!("Catalog page is not valid UTF-8: {error}"))
}

fn https_get(
    host: &str,
    path: &str,
    accept_header: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let host_w = wide_null(host);
    let path_w = wide_null(path);

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
    unsafe { WinHttpSetTimeouts(session.raw(), 2_000, 4_000, 4_000, 12_000) }
        .map_err(|error| format!("WinHttpSetTimeouts failed: {error}"))?;

    // SAFETY: `session` remains live, and `host_w` is a nul-terminated buffer
    // that outlives the synchronous connection creation call.
    let connection = unsafe {
        WinHttpHandle::from_raw(
            WinHttpConnect(
                session.raw(),
                PCWSTR(host_w.as_ptr()),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ),
            "WinHttpConnect",
        )?
    };

    // SAFETY: `connection` remains live, `path_w` is nul-terminated, and all
    // optional pointer arguments are intentionally null. The request is HTTPS.
    let request = unsafe {
        WinHttpHandle::from_raw(
            WinHttpOpenRequest(
                connection.raw(),
                w!("GET"),
                PCWSTR(path_w.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "WinHttpOpenRequest",
        )?
    };

    let headers: Vec<u16> = accept_header.encode_utf16().collect();
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
            "GitHub catalog returned unexpected HTTP status {status_code}"
        ));
    }

    let mut body = Vec::with_capacity(64 * 1024);
    let mut buffer = [0_u8; 16 * 1024];
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
        if body.len() + bytes_read > max_bytes {
            return Err("GitHub catalog response is too large".to_owned());
        }
        body.extend_from_slice(&buffer[..bytes_read]);
    }
    Ok(body)
}

fn extract_default_stratagem_db(html: &str) -> Result<String, String> {
    const MARKER: &str = "const defaultStratagemDB";
    let marker_at = html
        .find(MARKER)
        .ok_or_else(|| "Remote page does not contain defaultStratagemDB".to_owned())?;
    let after_marker = &html[marker_at + MARKER.len()..];
    let eq_at = after_marker
        .find('=')
        .ok_or_else(|| "Remote defaultStratagemDB assignment is malformed".to_owned())?;
    let mut rest = after_marker[eq_at + 1..].trim_start();
    if !rest.starts_with('[') {
        return Err("Remote defaultStratagemDB is not an array".to_owned());
    }

    let mut depth = 0_i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut end = None;
    for (index, ch) in rest.char_indices() {
        if in_single {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
            continue;
        }
        match ch {
            '\'' => in_single = true,
            '"' => in_double = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(index + ch.len_utf8());
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| "Remote defaultStratagemDB array is truncated".to_owned())?;
    rest = &rest[..end];
    Ok(rest.to_owned())
}

/// Convert a narrow JavaScript array/object literal (as used in this project's
/// `index.html`) into JSON text. Only supports the constructs that appear in
/// `defaultStratagemDB`.
fn js_array_literal_to_json(js: &str) -> Result<String, String> {
    let mut out = String::with_capacity(js.len() + js.len() / 8);
    let mut chars = js.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '\'' {
            let (segment, next) = read_single_quoted_string(js, index)?;
            out.push_str(&segment);
            // Advance the iterator to `next`.
            while chars.peek().is_some_and(|(i, _)| *i < next) {
                chars.next();
            }
            continue;
        }
        if ch == '"' {
            let (segment, next) = read_double_quoted_string(js, index)?;
            out.push_str(&segment);
            while chars.peek().is_some_and(|(i, _)| *i < next) {
                chars.next();
            }
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = index;
            let mut end = index + ch.len_utf8();
            while let Some(&(i, c)) = chars.peek() {
                if c.is_ascii_alphanumeric() || c == '_' {
                    end = i + c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let ident = &js[start..end];
            let mut look = end;
            while look < js.len() {
                let c = js[look..].chars().next().unwrap();
                if c.is_whitespace() {
                    look += c.len_utf8();
                } else {
                    break;
                }
            }
            if js[look..].starts_with(':') {
                out.push('"');
                out.push_str(ident);
                out.push('"');
            } else {
                out.push_str(ident);
            }
            continue;
        }
        // Drop trailing commas before } or ], and elided array holes such as
        // `aliases:[,'foo']` which appear in the published index.html.
        if ch == ',' {
            let mut look = index + 1;
            while look < js.len() {
                let c = js[look..].chars().next().unwrap();
                if c.is_whitespace() {
                    look += c.len_utf8();
                } else {
                    break;
                }
            }
            if js[look..].starts_with('}') || js[look..].starts_with(']') {
                continue;
            }
            let previous = out.chars().rev().find(|c| !c.is_whitespace());
            if matches!(previous, Some('[') | Some(',')) {
                // Leading/consecutive hole in an array: skip this comma.
                continue;
            }
            out.push(',');
            continue;
        }
        out.push(ch);
    }
    Ok(out)
}

fn read_single_quoted_string(src: &str, start: usize) -> Result<(String, usize), String> {
    if !src[start..].starts_with('\'') {
        return Err("Expected a single-quoted string".to_owned());
    }
    let mut out = String::from("\"");
    let mut escaped = false;
    for (rel, ch) in src[start + 1..].char_indices() {
        let absolute = start + 1 + rel;
        if escaped {
            match ch {
                '\'' | '\\' | '"' => out.push(ch),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => {
                    out.push('\\');
                    out.push(other);
                }
            }
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '\'' {
            out.push('"');
            return Ok((out, absolute + ch.len_utf8()));
        }
        if ch == '"' {
            out.push_str("\\\"");
            continue;
        }
        if ch == '\n' || ch == '\r' {
            return Err("Unterminated single-quoted string in catalog".to_owned());
        }
        // JSON requires control characters to be escaped.
        if ch.is_control() {
            out.push_str(&format!("\\u{:04x}", ch as u32));
        } else {
            out.push(ch);
        }
    }
    Err("Unterminated single-quoted string in catalog".to_owned())
}

fn read_double_quoted_string(src: &str, start: usize) -> Result<(String, usize), String> {
    if !src[start..].starts_with('"') {
        return Err("Expected a double-quoted string".to_owned());
    }
    let mut out = String::from("\"");
    let mut escaped = false;
    for (rel, ch) in src[start + 1..].char_indices() {
        let absolute = start + 1 + rel;
        out.push(ch);
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Ok((out, absolute + ch.len_utf8()));
        }
    }
    Err("Unterminated double-quoted string in catalog".to_owned())
}

fn validate_stratagem_catalog(value: &Value) -> Result<(), String> {
    let items = value
        .as_array()
        .ok_or_else(|| "Remote catalog must be a JSON array".to_owned())?;
    if items.is_empty() {
        return Err("Remote catalog is empty".to_owned());
    }
    if items.len() > MAX_STRATAGEMS {
        return Err(format!(
            "Remote catalog has too many entries ({})",
            items.len()
        ));
    }
    let mut seen = Map::new();
    for (index, item) in items.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| format!("Catalog entry {index} is not an object"))?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Catalog entry {index} is missing id"))?;
        if id.is_empty() || id.len() > MAX_ID_LEN || !id.chars().all(valid_id_char) {
            return Err(format!("Catalog entry {index} has an invalid id"));
        }
        if seen.insert(id.to_owned(), Value::Null).is_some() {
            return Err(format!("Catalog contains a duplicate id: {id}"));
        }
        let seq = object
            .get("seq")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("Catalog entry {id} is missing seq"))?;
        if seq.is_empty() || seq.len() > MAX_SEQ_LEN {
            return Err(format!("Catalog entry {id} has an invalid seq"));
        }
        for step in seq {
            let token = step
                .as_str()
                .ok_or_else(|| format!("Catalog entry {id} has a non-string seq step"))?;
            if !matches!(token, "W" | "A" | "S" | "D") {
                return Err(format!(
                    "Catalog entry {id} has an invalid seq step {token:?}"
                ));
            }
        }
        if object.get("grp").and_then(Value::as_str).is_none() {
            return Err(format!("Catalog entry {id} is missing grp"));
        }
        if object.get("name").and_then(Value::as_object).is_none() {
            return Err(format!("Catalog entry {id} is missing name"));
        }
    }
    Ok(())
}

fn valid_id_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        const defaultStratagemDB = [
            { id:'wpn_mg', grp:'support', name:{zh:'MG-43 Machine Gun',en:'MG-43 Machine Gun'}, aliases:[],ocr:['Machine Gun'], seq:['S','A','S','W','D'] , icon: 'Machine_Gun_Stratagem_Icon.svg'},
            { id:'mis_sos', grp:'mission', name:{zh:'SOS Beacon',en:'SOS Beacon'}, aliases:['sos'], seq:['W','S','D','W'] , icon: 'SOS_Beacon_Stratagem_Icon.svg'},
        ];
    "#;

    #[test]
    fn extracts_and_parses_sample_catalog() {
        let array = extract_default_stratagem_db(SAMPLE).unwrap();
        let json = js_array_literal_to_json(&array).unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        validate_stratagem_catalog(&value).unwrap();
        assert_eq!(value.as_array().unwrap().len(), 2);
        assert_eq!(value[0]["id"], "wpn_mg");
        assert_eq!(value[0]["name"]["zh"], "MG-43 Machine Gun");
        assert_eq!(value[1]["seq"][0], "W");
    }

    #[test]
    fn rejects_invalid_seq_tokens() {
        let bad = Value::Array(vec![serde_json::json!({
            "id": "bad",
            "grp": "support",
            "name": { "zh": "x", "en": "x" },
            "seq": ["W", "X"]
        })]);
        assert!(validate_stratagem_catalog(&bad).is_err());
    }

    #[test]
    fn parses_packaged_index_html_catalog() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui/index.html");
        let html = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let array = extract_default_stratagem_db(&html).expect("extract packaged catalog");
        let json = js_array_literal_to_json(&array).expect("convert packaged catalog");
        let value: Value = match serde_json::from_str(&json) {
            Ok(value) => value,
            Err(error) => {
                let line = error.line();
                let column = error.column();
                let snippet = json.lines().nth(line.saturating_sub(1)).unwrap_or_default();
                let chars: Vec<char> = snippet.chars().collect();
                let col = column.saturating_sub(1);
                let start = col.saturating_sub(40);
                let end = (col + 40).min(chars.len());
                let around: String = chars[start..end].iter().collect();
                panic!("json packaged catalog: {error}; around: {around:?}");
            }
        };
        validate_stratagem_catalog(&value).expect("validate packaged catalog");
        assert!(value.as_array().unwrap().len() > 50);
    }
}
