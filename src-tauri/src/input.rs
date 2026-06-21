use serde::Deserialize;
use std::{mem::size_of, thread, time::Duration};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, VIRTUAL_KEY, VK_ADD, VK_APPS,
    VK_BACK, VK_CAPITAL, VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1,
    VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MULTIPLY, VK_NEXT,
    VK_NUMPAD0, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA,
    VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU,
    VK_RSHIFT, VK_RWIN, VK_SPACE, VK_SUBTRACT, VK_TAB, VK_UP,
};

const MAPVK_VK_TO_VSC: u32 = 0;

extern "system" {
    fn MapVirtualKeyW(u_code: u32, u_map_type: u32) -> u32;
}

/// Returns true for virtual key codes that require the KEYEVENTF_EXTENDEDKEY
/// flag. Without it, arrow keys / navigation keys are indistinguishable from
/// their numpad equivalents when sending scancodes.
fn is_extended_key(vk: u16) -> bool {
    matches!(
        vk,
        0xA3 |  // VK_RCONTROL (right ctrl is extended, left is not)
        0xA5 |  // VK_RMENU (right alt is extended, left is not)
        0x21 |  // VK_PRIOR (PageUp)
        0x22 |  // VK_NEXT (PageDown)
        0x23 |  // VK_END
        0x24 |  // VK_HOME
        0x25 |  // VK_LEFT
        0x26 |  // VK_UP
        0x27 |  // VK_RIGHT
        0x28 |  // VK_DOWN
        0x2D |  // VK_INSERT
        0x2E |  // VK_DELETE
        0x5B |  // VK_LWIN
        0x5C |  // VK_RWIN
        0x6D // VK_DIVIDE (numpad / shares scancode with main /)
    )
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroPayload {
    pub menu_key: Option<String>,
    pub menu_mode: Option<String>,
    pub sequence: Vec<String>,
    pub menu_open_delay: Option<u64>,
    pub press_delay: Option<u64>,
    pub interval_delay: Option<u64>,
}

pub fn execute(payload: MacroPayload) {
    if payload.sequence.is_empty() {
        return;
    }

    thread::spawn(move || run_macro(payload));
}

fn run_macro(payload: MacroPayload) {
    let menu_key = payload
        .menu_key
        .as_deref()
        .and_then(virtual_key)
        .unwrap_or(VK_LCONTROL);
    let menu_open_delay = normalize_delay(payload.menu_open_delay, 150);
    let press_delay = normalize_delay(payload.press_delay, 15);
    let interval_delay = normalize_delay(payload.interval_delay, 15);
    let hold_mode = payload.menu_mode.as_deref() == Some("hold");

    let _ = key_up(menu_key);
    sleep(10);

    if hold_mode {
        let _ = key_down(menu_key);
    } else {
        let _ = key_down(menu_key);
        sleep(press_delay.saturating_add(20));
        let _ = key_up(menu_key);
    }

    sleep(menu_open_delay);
    for key_name in payload.sequence {
        if let Some(key) = virtual_key(&key_name) {
            let _ = key_down(key);
            sleep(press_delay);
            let _ = key_up(key);
            sleep(interval_delay);
        }
    }

    if hold_mode {
        sleep(50);
        let _ = key_up(menu_key);
    }
}

fn normalize_delay(value: Option<u64>, fallback: u64) -> u64 {
    value.filter(|delay| *delay > 0).unwrap_or(fallback)
}

fn sleep(milliseconds: u64) {
    thread::sleep(Duration::from_millis(milliseconds));
}

fn key_down(key: VIRTUAL_KEY) -> Result<(), String> {
    send_key(key, KEYBD_EVENT_FLAGS(0))
}

fn key_up(key: VIRTUAL_KEY) -> Result<(), String> {
    send_key(key, KEYEVENTF_KEYUP)
}

fn send_key(key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> Result<(), String> {
    // Use scancodes like nut-js does — many games (including HD2) ignore
    // SendInput events that only carry a virtual key code without a scancode.
    let vk = key.0;
    let scan_code = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    let mut full_flags = flags;
    if scan_code != 0 {
        full_flags |= KEYEVENTF_SCANCODE;
    }
    // Extended keys (arrows, navigation, right-side modifiers, NumpadEnter)
    // need KEYEVENTF_EXTENDEDKEY to be distinguishable from their numpad
    // equivalents when using scancodes.
    if is_extended_key(vk) {
        full_flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan_code,
                dwFlags: full_flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
    if sent == 1 {
        Ok(())
    } else {
        Err("Windows rejected synthetic keyboard input".to_owned())
    }
}

fn virtual_key(name: &str) -> Option<VIRTUAL_KEY> {
    let named_key = match name {
        "ControlLeft" => VK_LCONTROL,
        "ControlRight" => VK_RCONTROL,
        "ShiftLeft" => VK_LSHIFT,
        "ShiftRight" => VK_RSHIFT,
        "AltLeft" => VK_LMENU,
        "AltRight" => VK_RMENU,
        "MetaLeft" | "OSLeft" => VK_LWIN,
        "MetaRight" | "OSRight" => VK_RWIN,
        "ArrowUp" | "Up" => VK_UP,
        "ArrowDown" | "Down" => VK_DOWN,
        "ArrowLeft" | "Left" => VK_LEFT,
        "ArrowRight" | "Right" => VK_RIGHT,
        "NumpadAdd" => VK_ADD,
        "NumpadSubtract" => VK_SUBTRACT,
        "NumpadMultiply" => VK_MULTIPLY,
        "NumpadDivide" => VK_DIVIDE,
        "NumpadDecimal" => VK_DECIMAL,
        "NumpadEnter" => VK_RETURN,
        "Enter" => VK_RETURN,
        "Escape" => VK_ESCAPE,
        "Backspace" => VK_BACK,
        "Space" => VK_SPACE,
        "Tab" => VK_TAB,
        "CapsLock" => VK_CAPITAL,
        "PageUp" => VK_PRIOR,
        "PageDown" => VK_NEXT,
        "Home" => VK_HOME,
        "End" => VK_END,
        "Insert" => VK_INSERT,
        "Delete" => VK_DELETE,
        "Minus" => VK_OEM_MINUS,
        "Equal" => VK_OEM_PLUS,
        "BracketLeft" => VK_OEM_4,
        "BracketRight" => VK_OEM_6,
        "Semicolon" => VK_OEM_1,
        "Quote" => VK_OEM_7,
        "Backquote" => VK_OEM_3,
        "Backslash" => VK_OEM_5,
        "Comma" => VK_OEM_COMMA,
        "Period" => VK_OEM_PERIOD,
        "Slash" => VK_OEM_2,
        "ContextMenu" => VK_APPS,
        _ => return numeric_or_letter_key(name),
    };
    Some(named_key)
}

fn numeric_or_letter_key(name: &str) -> Option<VIRTUAL_KEY> {
    if let Some(number) = name
        .strip_prefix('F')
        .and_then(|value| value.parse::<u16>().ok())
    {
        return (1..=24)
            .contains(&number)
            .then_some(VIRTUAL_KEY(VK_F1.0 + number - 1));
    }
    if let Some(number) = name
        .strip_prefix("Numpad")
        .and_then(|value| value.parse::<u16>().ok())
    {
        return (0..=9)
            .contains(&number)
            .then_some(VIRTUAL_KEY(VK_NUMPAD0.0 + number));
    }
    if let Some(number) = name
        .strip_prefix("Digit")
        .and_then(|value| value.parse::<u16>().ok())
    {
        return (0..=9)
            .contains(&number)
            .then_some(VIRTUAL_KEY(b'0' as u16 + number));
    }
    if let Some(letter) = name.strip_prefix("Key") {
        return single_letter_key(letter);
    }
    if name.len() == 1 {
        return single_letter_key(name).or_else(|| {
            name.parse::<u16>().ok().and_then(|number| {
                (0..=9)
                    .contains(&number)
                    .then_some(VIRTUAL_KEY(b'0' as u16 + number))
            })
        });
    }
    None
}

fn single_letter_key(value: &str) -> Option<VIRTUAL_KEY> {
    let byte = value.as_bytes().first().copied()?.to_ascii_uppercase();
    (byte.is_ascii_uppercase()).then_some(VIRTUAL_KEY(byte as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_existing_renderer_key_names() {
        assert_eq!(virtual_key("KeyW"), Some(VIRTUAL_KEY(b'W' as u16)));
        assert_eq!(virtual_key("Digit4"), Some(VIRTUAL_KEY(b'4' as u16)));
        assert_eq!(virtual_key("F8"), Some(VIRTUAL_KEY(VK_F1.0 + 7)));
        assert_eq!(virtual_key("Numpad6"), Some(VIRTUAL_KEY(VK_NUMPAD0.0 + 6)));
        assert_eq!(virtual_key("NotAKey"), None);
    }

    #[test]
    fn keeps_the_electron_delay_defaults() {
        assert_eq!(normalize_delay(None, 150), 150);
        assert_eq!(normalize_delay(Some(0), 15), 15);
        assert_eq!(normalize_delay(Some(7), 15), 7);
    }
}
