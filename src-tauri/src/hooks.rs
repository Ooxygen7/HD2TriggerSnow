use std::{
    sync::{mpsc, OnceLock},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION,
        KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
        WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_SYSKEYDOWN,
        WM_XBUTTONDOWN,
    },
};

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static LISTENER_STARTED: OnceLock<()> = OnceLock::new();

pub fn start(app: AppHandle) -> Result<(), String> {
    if LISTENER_STARTED.get().is_some() {
        return Ok(());
    }
    APP_HANDLE
        .set(app)
        .map_err(|_| "Global input listener already owns a different app handle".to_owned())?;

    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("hd2-global-input".to_owned())
        .spawn(move || run_message_loop(ready_sender))
        .map_err(|error| format!("Cannot start global input listener: {error}"))?;

    ready_receiver
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "Timed out while registering the global input listener".to_owned())??;
    let _ = LISTENER_STARTED.set(());
    Ok(())
}

fn run_message_loop(ready_sender: mpsc::SyncSender<Result<(), String>>) {
    let keyboard_hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0) };
    let mouse_hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) };

    let hooks = match (keyboard_hook, mouse_hook) {
        (Ok(keyboard), Ok(mouse)) => {
            let _ = ready_sender.send(Ok(()));
            Some((keyboard, mouse))
        }
        (keyboard, mouse) => {
            if let Ok(hook) = keyboard {
                let _ = unsafe { UnhookWindowsHookEx(hook) };
            }
            if let Ok(hook) = mouse {
                let _ = unsafe { UnhookWindowsHookEx(hook) };
            }
            let _ = ready_sender.send(Err(
                "Windows refused to register global keyboard or mouse hooks".to_owned(),
            ));
            None
        }
    };

    if hooks.is_none() {
        return;
    }

    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {}

    if let Some((keyboard, mouse)) = hooks {
        let _ = unsafe { UnhookWindowsHookEx(keyboard) };
        let _ = unsafe { UnhookWindowsHookEx(mouse) };
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN)
        && lparam.0 != 0
    {
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if let Some(key_name) = key_name(event.vkCode, event.scanCode, event.flags.0) {
            emit("global-keydown", key_name);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        match wparam.0 as u32 {
            WM_MBUTTONDOWN => emit("global-mousedown", "MouseMiddle"),
            WM_XBUTTONDOWN => match event.mouseData & 0xffff {
                1 => emit("global-mousedown", "MouseSide1"),
                2 => emit("global-mousedown", "MouseSide2"),
                _ => {}
            },
            WM_MOUSEWHEEL => {
                let rotation = (event.mouseData >> 16) as i16;
                if rotation != 0 {
                    // uIOhook (used by the Electron version) reports wheel-up as
                    // negative rotation. WM_MOUSEWHEEL reports wheel-up as positive.
                    // Negate to match the legacy contract the frontend expects.
                    emit("global-wheel", if rotation > 0 { -1 } else { 1 });
                }
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {}
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn emit<S: serde::Serialize + Clone>(event: &str, payload: S) {
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit_to("main", event, payload);
    }
}

fn key_name(vk: u32, scan_code: u32, flags: u32) -> Option<&'static str> {
    let extended = flags & 1 != 0;
    match vk {
        0x41 => Some("KeyA"),
        0x42 => Some("KeyB"),
        0x43 => Some("KeyC"),
        0x44 => Some("KeyD"),
        0x45 => Some("KeyE"),
        0x46 => Some("KeyF"),
        0x47 => Some("KeyG"),
        0x48 => Some("KeyH"),
        0x49 => Some("KeyI"),
        0x4a => Some("KeyJ"),
        0x4b => Some("KeyK"),
        0x4c => Some("KeyL"),
        0x4d => Some("KeyM"),
        0x4e => Some("KeyN"),
        0x4f => Some("KeyO"),
        0x50 => Some("KeyP"),
        0x51 => Some("KeyQ"),
        0x52 => Some("KeyR"),
        0x53 => Some("KeyS"),
        0x54 => Some("KeyT"),
        0x55 => Some("KeyU"),
        0x56 => Some("KeyV"),
        0x57 => Some("KeyW"),
        0x58 => Some("KeyX"),
        0x59 => Some("KeyY"),
        0x5a => Some("KeyZ"),
        0x30 => Some("Digit0"),
        0x31 => Some("Digit1"),
        0x32 => Some("Digit2"),
        0x33 => Some("Digit3"),
        0x34 => Some("Digit4"),
        0x35 => Some("Digit5"),
        0x36 => Some("Digit6"),
        0x37 => Some("Digit7"),
        0x38 => Some("Digit8"),
        0x39 => Some("Digit9"),
        0x60 => Some("Numpad0"),
        0x61 => Some("Numpad1"),
        0x62 => Some("Numpad2"),
        0x63 => Some("Numpad3"),
        0x64 => Some("Numpad4"),
        0x65 => Some("Numpad5"),
        0x66 => Some("Numpad6"),
        0x67 => Some("Numpad7"),
        0x68 => Some("Numpad8"),
        0x69 => Some("Numpad9"),
        0x6a => Some("NumpadMultiply"),
        0x6b => Some("NumpadAdd"),
        0x6d => Some("NumpadSubtract"),
        0x6e => Some("NumpadDecimal"),
        0x6f => Some("NumpadDivide"),
        0x70 => Some("F1"),
        0x71 => Some("F2"),
        0x72 => Some("F3"),
        0x73 => Some("F4"),
        0x74 => Some("F5"),
        0x75 => Some("F6"),
        0x76 => Some("F7"),
        0x77 => Some("F8"),
        0x78 => Some("F9"),
        0x79 => Some("F10"),
        0x7a => Some("F11"),
        0x7b => Some("F12"),
        0x20 => Some("Space"),
        0x09 => Some("Tab"),
        0x14 => Some("CapsLock"),
        0x1b => Some("Escape"),
        0x08 => Some("Backspace"),
        0x0d if extended => Some("NumpadEnter"),
        0x0d => Some("Enter"),
        0xa0 => Some("ShiftLeft"),
        0xa1 => Some("ShiftRight"),
        0xa2 => Some("ControlLeft"),
        0xa3 => Some("ControlRight"),
        0xa4 => Some("AltLeft"),
        0xa5 => Some("AltRight"),
        0x5b => Some("MetaLeft"),
        0x5c => Some("MetaRight"),
        0x10 if scan_code == 0x36 => Some("ShiftRight"),
        0x10 => Some("ShiftLeft"),
        0x11 if extended => Some("ControlRight"),
        0x11 => Some("ControlLeft"),
        0x12 if extended => Some("AltRight"),
        0x12 => Some("AltLeft"),
        0xbd => Some("Minus"),
        0xbb => Some("Equal"),
        0xdb => Some("BracketLeft"),
        0xdd => Some("BracketRight"),
        0xba => Some("Semicolon"),
        0xde => Some("Quote"),
        0xc0 => Some("Backquote"),
        0xdc => Some("Backslash"),
        0xbc => Some("Comma"),
        0xbe => Some("Period"),
        0xbf => Some("Slash"),
        0x21 => Some("PageUp"),
        0x22 => Some("PageDown"),
        0x24 => Some("Home"),
        0x23 => Some("End"),
        0x2d => Some("Insert"),
        0x2e => Some("Delete"),
        0x25 => Some("ArrowLeft"),
        0x26 => Some("ArrowUp"),
        0x27 => Some("ArrowRight"),
        0x28 => Some("ArrowDown"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_legacy_key_names() {
        assert_eq!(key_name(0x57, 0x11, 0), Some("KeyW"));
        assert_eq!(key_name(0x77, 0x42, 0), Some("F8"));
        assert_eq!(key_name(0x11, 0x1d, 1), Some("ControlRight"));
        assert_eq!(key_name(0x0d, 0x1c, 1), Some("NumpadEnter"));
        assert_eq!(key_name(0x05, 0, 0), None);
    }
}
