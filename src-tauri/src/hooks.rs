use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        mpsc, Mutex, OnceLock,
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
        HC_ACTION, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
        WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEWHEEL, WM_QUIT,
        WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
    },
};

static LISTENER_STARTED: OnceLock<()> = OnceLock::new();
static EVENT_SENDER: OnceLock<mpsc::SyncSender<InputEvent>> = OnceLock::new();
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static DROPPED_EVENTS: AtomicU64 = AtomicU64::new(0);
static FILTER_INITIALIZED: AtomicBool = AtomicBool::new(false);
static CAPTURE_ALL_INPUTS: AtomicBool = AtomicBool::new(true);
static KEY_FILTER: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static PRESSED_KEYS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static PRESSED_MOUSE: AtomicU8 = AtomicU8::new(0);
static MOUSE_FILTER: AtomicU8 = AtomicU8::new(0);
static WHEEL_FILTER: AtomicBool = AtomicBool::new(false);
static FILTER_UPDATE_LOCK: Mutex<()> = Mutex::new(());

const LLKHF_INJECTED_FLAG: u32 = 0x10;
const LLMHF_INJECTED_FLAG: u32 = 0x01;
const FILTER_MOUSE_MIDDLE: u8 = 1 << 0;
const FILTER_MOUSE_SIDE1: u8 = 1 << 1;
const FILTER_MOUSE_SIDE2: u8 = 1 << 2;
const PRESSED_NUMPAD_ENTER: u32 = 0xff;

#[derive(Default)]
struct InputFilter {
    keys: [u64; 4],
    mouse: u8,
    wheel: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PressedInputs {
    keys: [u64; 4],
    mouse: u8,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GlobalInputPayload {
    key: &'static str,
    pressed_inputs: Vec<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputEvent {
    KeyDown(&'static str, PressedInputs),
    MouseDown(&'static str, PressedInputs),
    Wheel(&'static str, PressedInputs),
    Shutdown,
}

pub fn start(app: AppHandle) -> Result<(), String> {
    if LISTENER_STARTED.get().is_some() {
        return Ok(());
    }
    reset_pressed_inputs();
    let (event_sender, event_receiver) = mpsc::sync_channel(1024);
    EVENT_SENDER
        .set(event_sender)
        .map_err(|_| "Global input event queue is already initialized".to_owned())?;
    thread::Builder::new()
        .name("hd2-input-events".to_owned())
        .stack_size(512 * 1024)
        .spawn(move || forward_events(app, event_receiver))
        .map_err(|error| format!("Cannot start global input event worker: {error}"))?;

    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("hd2-global-input".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || run_message_loop(ready_sender))
        .map_err(|error| format!("Cannot start global input listener: {error}"))?;

    ready_receiver
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "Timed out while registering the global input listener".to_owned())??;
    let _ = LISTENER_STARTED.set(());
    Ok(())
}

pub fn stop() {
    if let Some(sender) = EVENT_SENDER.get() {
        let _ = sender.try_send(InputEvent::Shutdown);
    }
    let thread_id = HOOK_THREAD_ID.swap(0, Ordering::AcqRel);
    if thread_id != 0 {
        // SAFETY: `thread_id` was published by the hook thread from
        // `GetCurrentThreadId`. `WM_QUIT` carries no pointer payload, so both
        // integer message parameters remain valid even if the target thread
        // exits concurrently (in which case Windows simply returns an error).
        let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

pub fn update_filter(keys: &[String], capture_all: bool) -> Result<(), String> {
    if keys.len() > 128 || keys.iter().any(|key| key.len() > 64) {
        return Err("The global input filter is too large".to_owned());
    }
    let filter = build_filter(keys)?;
    let _writer = FILTER_UPDATE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    // Forward everything while the multi-atomic snapshot is replaced. This
    // can produce at most a harmless extra event and never temporarily hides a
    // requested binding. The writer lock prevents overlapping snapshots.
    CAPTURE_ALL_INPUTS.store(true, Ordering::Release);
    for (destination, value) in KEY_FILTER.iter().zip(filter.keys) {
        destination.store(value, Ordering::Release);
    }
    MOUSE_FILTER.store(filter.mouse, Ordering::Release);
    WHEEL_FILTER.store(filter.wheel, Ordering::Release);
    FILTER_INITIALIZED.store(true, Ordering::Release);
    CAPTURE_ALL_INPUTS.store(capture_all, Ordering::Release);
    Ok(())
}

fn build_filter(keys: &[String]) -> Result<InputFilter, String> {
    let mut filter = InputFilter::default();
    for key in keys {
        match key.as_str() {
            "MouseMiddle" => filter.mouse |= FILTER_MOUSE_MIDDLE,
            "MouseSide1" => filter.mouse |= FILTER_MOUSE_SIDE1,
            "MouseSide2" => filter.mouse |= FILTER_MOUSE_SIDE2,
            "WheelUp" | "WheelDown" => filter.wheel = true,
            name => {
                let code = crate::input::filter_virtual_key(name)
                    .ok_or_else(|| format!("Unsupported global input binding: {name}"))?;
                insert_filtered_key(&mut filter.keys, code);
                // Low-level hooks may report side-specific modifiers as their
                // generic VK_* value. Accept both representations.
                match code {
                    0xA0 | 0xA1 => insert_filtered_key(&mut filter.keys, 0x10),
                    0xA2 | 0xA3 => insert_filtered_key(&mut filter.keys, 0x11),
                    0xA4 | 0xA5 => insert_filtered_key(&mut filter.keys, 0x12),
                    _ => {}
                }
            }
        }
    }
    Ok(filter)
}

fn insert_filtered_key(filter: &mut [u64; 4], code: u16) {
    let code = usize::from(code);
    filter[code / 64] |= 1_u64 << (code % 64);
}

fn forwards_all_inputs() -> bool {
    !FILTER_INITIALIZED.load(Ordering::Acquire) || CAPTURE_ALL_INPUTS.load(Ordering::Acquire)
}

fn key_is_relevant(virtual_key: u32) -> bool {
    if forwards_all_inputs() {
        return true;
    }
    let Ok(virtual_key) = usize::try_from(virtual_key) else {
        return false;
    };
    KEY_FILTER
        .get(virtual_key / 64)
        .is_some_and(|word| word.load(Ordering::Acquire) & (1_u64 << (virtual_key % 64)) != 0)
}

fn mouse_is_relevant(mask: u8) -> bool {
    forwards_all_inputs() || MOUSE_FILTER.load(Ordering::Acquire) & mask != 0
}

fn wheel_is_relevant() -> bool {
    forwards_all_inputs() || WHEEL_FILTER.load(Ordering::Acquire)
}

fn reset_pressed_inputs() {
    for word in &PRESSED_KEYS {
        word.store(0, Ordering::Release);
    }
    PRESSED_MOUSE.store(0, Ordering::Release);
}

fn mark_key_pressed(virtual_key: u32) -> bool {
    let Ok(virtual_key) = usize::try_from(virtual_key) else {
        return false;
    };
    let Some(word) = PRESSED_KEYS.get(virtual_key / 64) else {
        return false;
    };
    let mask = 1_u64 << (virtual_key % 64);
    word.fetch_or(mask, Ordering::AcqRel) & mask == 0
}

fn mark_key_released(virtual_key: u32) {
    let Ok(virtual_key) = usize::try_from(virtual_key) else {
        return;
    };
    if let Some(word) = PRESSED_KEYS.get(virtual_key / 64) {
        word.fetch_and(!(1_u64 << (virtual_key % 64)), Ordering::AcqRel);
    }
}

fn mark_mouse_pressed(mask: u8) -> bool {
    PRESSED_MOUSE.fetch_or(mask, Ordering::AcqRel) & mask == 0
}

fn mark_mouse_released(mask: u8) {
    PRESSED_MOUSE.fetch_and(!mask, Ordering::AcqRel);
}

fn pressed_state_code(key: &str, fallback: u32) -> u32 {
    if key == "NumpadEnter" {
        // Main Enter and NumpadEnter share VK_RETURN. Reserve an otherwise
        // unused bit so either key can participate in a chord independently.
        PRESSED_NUMPAD_ENTER
    } else {
        crate::input::filter_virtual_key(key)
            .map(u32::from)
            .unwrap_or(fallback)
    }
}

fn snapshot_pressed_inputs() -> PressedInputs {
    let mut snapshot = PressedInputs::default();
    for (destination, source) in snapshot.keys.iter_mut().zip(&PRESSED_KEYS) {
        *destination = source.load(Ordering::Acquire);
    }
    snapshot.mouse = PRESSED_MOUSE.load(Ordering::Acquire);
    snapshot
}

fn global_input_payload(key: &'static str, snapshot: PressedInputs) -> GlobalInputPayload {
    let mut pressed_inputs = Vec::with_capacity(10);
    for (word_index, word) in snapshot.keys.into_iter().enumerate() {
        let mut remaining = word;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            let code = (word_index * 64 + bit) as u32;
            let name = if code == PRESSED_NUMPAD_ENTER {
                Some("NumpadEnter")
            } else {
                key_name(code, 0, 0)
            };
            if let Some(name) = name {
                pressed_inputs.push(name);
            }
            remaining &= remaining - 1;
        }
    }
    for (mask, name) in [
        (FILTER_MOUSE_MIDDLE, "MouseMiddle"),
        (FILTER_MOUSE_SIDE1, "MouseSide1"),
        (FILTER_MOUSE_SIDE2, "MouseSide2"),
    ] {
        if snapshot.mouse & mask != 0 {
            pressed_inputs.push(name);
        }
    }
    // Keep the edge that caused this event last. The frontend treats that
    // final key as the chord trigger while all preceding keys are requirements.
    pressed_inputs.retain(|pressed| *pressed != key);
    pressed_inputs.push(key);
    GlobalInputPayload {
        key,
        pressed_inputs,
    }
}

fn forward_events(app: AppHandle, receiver: mpsc::Receiver<InputEvent>) {
    while let Ok(event) = receiver.recv() {
        let _ = match event {
            InputEvent::KeyDown(key, snapshot) => app.emit_to(
                "main",
                "global-keydown",
                global_input_payload(key, snapshot),
            ),
            InputEvent::MouseDown(button, snapshot) => app.emit_to(
                "main",
                "global-mousedown",
                global_input_payload(button, snapshot),
            ),
            InputEvent::Wheel(direction, snapshot) => app.emit_to(
                "main",
                "global-wheel",
                global_input_payload(direction, snapshot),
            ),
            InputEvent::Shutdown => break,
        };
    }
}

fn run_message_loop(ready_sender: mpsc::SyncSender<Result<(), String>>) {
    // SAFETY: `keyboard_hook` has the required `extern "system"` ABI and a
    // process-static address. A null module handle and thread id zero are the
    // documented parameters for a process-hosted low-level global hook.
    let keyboard_hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0) };
    // SAFETY: `mouse_hook` has the required `extern "system"` ABI and a
    // process-static address. A null module handle and thread id zero are the
    // documented parameters for a process-hosted low-level global hook.
    let mouse_hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) };

    let hooks = match (keyboard_hook, mouse_hook) {
        (Ok(keyboard), Ok(mouse)) => {
            // SAFETY: `GetCurrentThreadId` takes no arguments and has no caller
            // preconditions; it returns the id of this message-loop thread.
            let thread_id = unsafe { GetCurrentThreadId() };
            HOOK_THREAD_ID.store(thread_id, Ordering::Release);
            let _ = ready_sender.send(Ok(()));
            Some((keyboard, mouse))
        }
        (keyboard, mouse) => {
            if let Ok(hook) = keyboard {
                // SAFETY: `hook` is the live handle returned by the successful
                // `SetWindowsHookExW` call above and has not been unhooked yet.
                let _ = unsafe { UnhookWindowsHookEx(hook) };
            }
            if let Ok(hook) = mouse {
                // SAFETY: `hook` is the live handle returned by the successful
                // `SetWindowsHookExW` call above and has not been unhooked yet.
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
    loop {
        // SAFETY: `message` is a valid writable `MSG` for the duration of the
        // call. A null HWND and zero filter bounds request all messages for the
        // current thread's queue, which is the documented message-loop form.
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result > 0 {
            continue;
        }
        // GetMessageW returns 0 for WM_QUIT and -1 for an API failure. Both
        // must terminate the listener; treating -1 as BOOL(true) spins forever.
        break;
    }
    HOOK_THREAD_ID.store(0, Ordering::Release);

    if let Some((keyboard, mouse)) = hooks {
        // SAFETY: both handles came from successful registrations above, are
        // still owned by this thread, and each is released exactly once here.
        let _ = unsafe { UnhookWindowsHookEx(keyboard) };
        // SAFETY: `mouse` is the second still-live handle from the same owned
        // pair and has not been passed to `UnhookWindowsHookEx` previously.
        let _ = unsafe { UnhookWindowsHookEx(mouse) };
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        // SAFETY: for an `HC_ACTION` low-level keyboard callback Windows
        // specifies that `lparam` points to a `KBDLLHOOKSTRUCT` which remains
        // valid for the duration of this callback; the null case was excluded.
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if !is_injected_keyboard(event.flags.0) {
            if let Some(key) = key_name(event.vkCode, event.scanCode, event.flags.0) {
                let state_code = pressed_state_code(key, event.vkCode);
                match wparam.0 as u32 {
                    WM_KEYUP | WM_SYSKEYUP => mark_key_released(state_code),
                    WM_KEYDOWN | WM_SYSKEYDOWN
                        if mark_key_pressed(state_code) && key_is_relevant(event.vkCode) =>
                    {
                        queue_event(InputEvent::KeyDown(key, snapshot_pressed_inputs()));
                    }
                    _ => {}
                }
            }
        }
    }
    // SAFETY: these are the unchanged callback arguments supplied by Windows;
    // passing `None` is supported because the hook handle parameter is ignored.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        // SAFETY: for an `HC_ACTION` low-level mouse callback Windows specifies
        // that `lparam` points to an `MSLLHOOKSTRUCT` valid until this callback
        // returns; the null case was explicitly excluded above.
        let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if is_injected_mouse(event.flags) {
            // SAFETY: these are the unchanged arguments supplied by Windows;
            // the hook handle is ignored by `CallNextHookEx`, so `None` is valid.
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        match wparam.0 as u32 {
            WM_MBUTTONDOWN => {
                let is_new = mark_mouse_pressed(FILTER_MOUSE_MIDDLE);
                if is_new && mouse_is_relevant(FILTER_MOUSE_MIDDLE) {
                    queue_event(InputEvent::MouseDown(
                        "MouseMiddle",
                        snapshot_pressed_inputs(),
                    ));
                }
            }
            WM_MBUTTONUP => mark_mouse_released(FILTER_MOUSE_MIDDLE),
            WM_XBUTTONDOWN => {
                if let Some(button) = side_button_name(event.mouseData) {
                    let mask = if button == "MouseSide1" {
                        FILTER_MOUSE_SIDE1
                    } else {
                        FILTER_MOUSE_SIDE2
                    };
                    let is_new = mark_mouse_pressed(mask);
                    if is_new && mouse_is_relevant(mask) {
                        queue_event(InputEvent::MouseDown(button, snapshot_pressed_inputs()));
                    }
                }
            }
            WM_XBUTTONUP => {
                if let Some(button) = side_button_name(event.mouseData) {
                    mark_mouse_released(if button == "MouseSide1" {
                        FILTER_MOUSE_SIDE1
                    } else {
                        FILTER_MOUSE_SIDE2
                    });
                }
            }
            WM_MOUSEWHEEL if wheel_is_relevant() => {
                let rotation = (event.mouseData >> 16) as i16;
                if rotation != 0 {
                    queue_event(InputEvent::Wheel(
                        if rotation > 0 { "WheelUp" } else { "WheelDown" },
                        snapshot_pressed_inputs(),
                    ));
                }
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {}
            _ => {}
        }
    }
    // SAFETY: these are the unchanged callback arguments supplied by Windows;
    // passing `None` is supported because the hook handle parameter is ignored.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// WM_XBUTTONDOWN stores XBUTTON1/XBUTTON2 in the high word of `mouseData`.
/// Reading the low word makes every side-button press look like zero, which
/// prevented the frontend from ever receiving the binding event.
fn side_button_name(mouse_data: u32) -> Option<&'static str> {
    match (mouse_data >> 16) & 0xffff {
        1 => Some("MouseSide1"),
        2 => Some("MouseSide2"),
        _ => None,
    }
}

fn queue_event(event: InputEvent) {
    if let Some(sender) = EVENT_SENDER.get() {
        if sender.try_send(event).is_err() {
            DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn is_injected_keyboard(flags: u32) -> bool {
    flags & LLKHF_INJECTED_FLAG != 0
}

fn is_injected_mouse(flags: u32) -> bool {
    flags & LLMHF_INJECTED_FLAG != 0
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
        0x7c => Some("F13"),
        0x7d => Some("F14"),
        0x7e => Some("F15"),
        0x7f => Some("F16"),
        0x80 => Some("F17"),
        0x81 => Some("F18"),
        0x82 => Some("F19"),
        0x83 => Some("F20"),
        0x84 => Some("F21"),
        0x85 => Some("F22"),
        0x86 => Some("F23"),
        0x87 => Some("F24"),
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
        0x5d => Some("ContextMenu"),
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
        assert_eq!(key_name(0x87, 0x76, 0), Some("F24"));
        assert_eq!(key_name(0x5d, 0x5d, 1), Some("ContextMenu"));
        assert_eq!(key_name(0x11, 0x1d, 1), Some("ControlRight"));
        assert_eq!(key_name(0x0d, 0x1c, 1), Some("NumpadEnter"));
        assert_eq!(key_name(0x05, 0, 0), None);
    }

    #[test]
    fn reads_xbutton_values_from_the_high_word() {
        assert_eq!(side_button_name(1 << 16), Some("MouseSide1"));
        assert_eq!(side_button_name(2 << 16), Some("MouseSide2"));
        assert_eq!(side_button_name(1), None);
    }

    #[test]
    fn builds_a_compact_filter_for_bound_global_inputs() {
        let filter = build_filter(&[
            "KeyW".to_owned(),
            "F8".to_owned(),
            "MouseSide1".to_owned(),
            "WheelUp".to_owned(),
        ])
        .expect("valid filter");
        let key_w = usize::from(crate::input::filter_virtual_key("KeyW").expect("KeyW"));
        let f8 = usize::from(crate::input::filter_virtual_key("F8").expect("F8"));
        assert_ne!(filter.keys[key_w / 64] & (1_u64 << (key_w % 64)), 0);
        assert_ne!(filter.keys[f8 / 64] & (1_u64 << (f8 % 64)), 0);
        assert_eq!(filter.mouse, FILTER_MOUSE_SIDE1);
        assert!(filter.wheel);
    }

    #[test]
    fn rejects_unknown_filter_keys_and_suppresses_key_repeat() {
        assert!(build_filter(&["NotARealKey".to_owned()]).is_err());

        let _state_guard = FILTER_UPDATE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reset_pressed_inputs();
        assert!(mark_key_pressed(0x57));
        assert!(!mark_key_pressed(0x57));
        mark_key_released(0x57);
        assert!(mark_key_pressed(0x57));
        mark_key_released(0x57);
    }

    #[test]
    fn snapshots_held_inputs_and_keeps_the_trigger_last() {
        let _state_guard = FILTER_UPDATE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reset_pressed_inputs();
        assert!(mark_key_pressed(pressed_state_code("ControlLeft", 0x11)));
        assert!(mark_key_pressed(pressed_state_code("KeyW", 0x57)));
        assert!(mark_mouse_pressed(FILTER_MOUSE_SIDE1));

        let payload = global_input_payload("Digit1", snapshot_pressed_inputs());
        assert_eq!(payload.key, "Digit1");
        assert!(payload.pressed_inputs.contains(&"ControlLeft"));
        assert!(payload.pressed_inputs.contains(&"KeyW"));
        assert!(payload.pressed_inputs.contains(&"MouseSide1"));
        assert_eq!(payload.pressed_inputs.last(), Some(&"Digit1"));

        mark_key_released(pressed_state_code("ControlLeft", 0x11));
        mark_key_released(pressed_state_code("KeyW", 0x57));
        mark_mouse_released(FILTER_MOUSE_SIDE1);
    }

    #[test]
    fn tracks_main_and_numpad_enter_as_distinct_chord_inputs() {
        assert_eq!(pressed_state_code("Enter", 0x0d), 0x0d);
        assert_eq!(
            pressed_state_code("NumpadEnter", 0x0d),
            PRESSED_NUMPAD_ENTER
        );
    }

    #[test]
    fn ignores_input_generated_by_send_input() {
        assert!(is_injected_keyboard(LLKHF_INJECTED_FLAG));
        assert!(is_injected_mouse(LLMHF_INJECTED_FLAG));
        assert!(!is_injected_keyboard(0));
        assert!(!is_injected_mouse(0));
    }
}
