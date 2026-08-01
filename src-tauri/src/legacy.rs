use std::mem::size_of;

use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
        },
    },
};

const LEGACY_EXECUTABLE_NAMES: &[&str] = &["HD2 Macro Terminal.exe", "HD2-Trigger.exe"];

struct ProcessSnapshot(HANDLE);

impl Drop for ProcessSnapshot {
    fn drop(&mut self) {
        // SAFETY: The handle is owned by this guard and is closed exactly once here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn executable_name(raw: &[u16]) -> String {
    let length = raw.iter().position(|unit| *unit == 0).unwrap_or(raw.len());
    String::from_utf16_lossy(&raw[..length])
}

fn is_legacy_executable_name(name: &str) -> bool {
    LEGACY_EXECUTABLE_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

fn running_legacy_process() -> Result<Option<(u32, String)>, String> {
    // SAFETY: The API receives a valid flag and does not dereference caller-owned pointers.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map(ProcessSnapshot)
        .map_err(|error| format!("could not enumerate running processes: {error}"))?;

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    // SAFETY: `entry` has the documented size and remains valid for the duration of the call.
    unsafe { Process32FirstW(snapshot.0, &mut entry) }
        .map_err(|error| format!("could not read the first running process: {error}"))?;

    loop {
        let name = executable_name(&entry.szExeFile);
        if entry.th32ProcessID != std::process::id() && is_legacy_executable_name(&name) {
            return Ok(Some((entry.th32ProcessID, name)));
        }

        // SAFETY: `entry` and the snapshot handle remain valid while the guard is alive.
        if unsafe { Process32NextW(snapshot.0, &mut entry) }.is_err() {
            break;
        }
    }

    Ok(None)
}

fn show_legacy_process_warning(process_name: &str) {
    let message = format!(
        "检测到旧版程序仍在运行：{process_name}\n\n\
         它可能已经隐藏到系统托盘。请先在旧版托盘菜单中选择“完全退出”，然后重新启动 Rust 版。\n\n\
         为避免两个重叠窗口、悬浮窗无法拖动和重复热键，本次启动已取消。\n\n\
         Legacy Electron version is still running. Exit it from the system tray, then start the Rust version again."
    );
    let message_wide: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title_wide: Vec<u16> = "HD2 Macro Terminal Rust - 旧版程序仍在运行"
        .encode_utf16()
        .chain(Some(0))
        .collect();

    // SAFETY: Both UTF-16 buffers are null-terminated and live until MessageBoxW returns.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

pub(crate) fn preflight_allows_startup() -> bool {
    match running_legacy_process() {
        Ok(Some((_process_id, process_name))) => {
            show_legacy_process_warning(&process_name);
            false
        }
        Ok(None) => true,
        Err(error) => {
            eprintln!("Legacy process preflight skipped: {error}");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_packaged_legacy_executable_names_case_insensitively() {
        assert!(is_legacy_executable_name("HD2 Macro Terminal.exe"));
        assert!(is_legacy_executable_name("hd2 macro terminal.EXE"));
        assert!(is_legacy_executable_name("HD2-Trigger.exe"));
    }

    #[test]
    fn does_not_block_rust_or_unrelated_electron_programs() {
        assert!(!is_legacy_executable_name("hd2-macro-terminal-rust.exe"));
        assert!(!is_legacy_executable_name("electron.exe"));
        assert!(!is_legacy_executable_name("HD2Arsenal.exe"));
    }

    #[test]
    fn decodes_a_null_terminated_process_name() {
        assert_eq!(
            executable_name(&['H' as u16, 'D' as u16, '2' as u16, 0, 'X' as u16]),
            "HD2"
        );
    }
}
