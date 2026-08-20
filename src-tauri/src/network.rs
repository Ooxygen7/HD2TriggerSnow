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
        // SAFETY: the handle is non-null, owned by this RAII value and closed
        // exactly once after all dependent handles have already been dropped.
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

pub fn fetch_https(
    host: &str,
    path: &str,
    accept: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    if host.is_empty()
        || host.len() > 253
        || !host.is_ascii()
        || host.contains(['/', '\\', ':', '\r', '\n'])
    {
        return Err("HTTPS host is invalid".to_owned());
    }
    if !path.starts_with('/') || path.len() > 2048 || path.contains(['\r', '\n']) {
        return Err("HTTPS path is invalid".to_owned());
    }
    if accept.is_empty()
        || accept.len() > 256
        || !accept.is_ascii()
        || accept.contains(['\r', '\n'])
    {
        return Err("HTTPS Accept header is invalid".to_owned());
    }
    if max_bytes == 0 || max_bytes > 16 * 1024 * 1024 {
        return Err("HTTPS response limit is invalid".to_owned());
    }

    let host = wide_null(host);
    let path = wide_null(path);
    // SAFETY: compile-time and owned UTF-16 strings remain valid for every
    // synchronous WinHTTP call. Each returned handle is checked and owned.
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
    // SAFETY: the live session accepts finite timeout values and retains no
    // borrowed Rust memory.
    unsafe { WinHttpSetTimeouts(session.raw(), 1_500, 2_500, 2_500, 4_000) }
        .map_err(|error| format!("WinHttpSetTimeouts failed: {error}"))?;
    // SAFETY: the session and NUL-terminated host are live for this call.
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
    // SAFETY: the connection and NUL-terminated path are live, optional
    // pointer arguments are null, and the secure flag requires HTTPS.
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
    let headers: Vec<u16> = format!("Accept: {accept}\r\n").encode_utf16().collect();
    // SAFETY: request and header storage are live for the synchronous call;
    // there is no body or asynchronous callback context.
    unsafe { WinHttpSendRequest(request.raw(), Some(&headers), None, 0, 0, 0) }
        .map_err(|error| format!("WinHttpSendRequest failed: {error}"))?;
    // SAFETY: request is live and the reserved pointer is null as required.
    unsafe { WinHttpReceiveResponse(request.raw(), ptr::null_mut()) }
        .map_err(|error| format!("WinHttpReceiveResponse failed: {error}"))?;

    let mut status_code = 0_u32;
    let mut status_length = size_of::<u32>() as u32;
    let mut header_index = 0_u32;
    // SAFETY: all output pointers reference valid writable u32 storage and
    // numeric status-code mode writes exactly one u32.
    unsafe {
        WinHttpQueryHeaders(
            request.raw(),
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut status_code).cast()),
            &mut status_length,
            &mut header_index,
        )
    }
    .map_err(|error| format!("WinHttpQueryHeaders failed: {error}"))?;
    if status_code != 200 {
        return Err(format!("HTTPS endpoint returned status {status_code}"));
    }

    let mut body = Vec::with_capacity(max_bytes.min(8 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let mut bytes_read = 0_u32;
        // SAFETY: request is live and buffer is valid writable storage for its
        // declared length; WinHTTP writes the actual count to bytes_read.
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
            return Err("HTTPS response exceeds the size limit".to_owned());
        }
        body.extend_from_slice(&buffer[..bytes_read]);
    }
    Ok(body)
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_header_and_path_injection_before_network_access() {
        assert!(fetch_https("example.com\r\nInjected", "/", "application/json", 10).is_err());
        assert!(fetch_https("example.com", "/ok\r\nInjected", "application/json", 10).is_err());
        assert!(fetch_https("example.com", "/", "application/json\r\nInjected", 10).is_err());
    }
}
