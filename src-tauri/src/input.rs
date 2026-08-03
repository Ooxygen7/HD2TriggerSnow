use serde::Deserialize;
use std::{
    mem::size_of,
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, MOUSE_EVENT_FLAGS,
    VIRTUAL_KEY, VK_ADD, VK_APPS, VK_BACK, VK_CAPITAL, VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN,
    VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT,
    VK_LWIN, VK_MULTIPLY, VK_NEXT, VK_NUMPAD0, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5,
    VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_PRIOR,
    VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SPACE, VK_SUBTRACT, VK_TAB,
    VK_UP,
};

static MACRO_RUNNING: AtomicBool = AtomicBool::new(false);
static MACRO_CANCELLED: AtomicBool = AtomicBool::new(false);
static MACRO_SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static MACRO_SIGNAL: OnceLock<(Mutex<()>, Condvar)> = OnceLock::new();
const MAX_DELAY_MS: u64 = 60_000;
const MAX_MACRO_DURATION_MS: u64 = 300_000;

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
        0x5D |  // VK_APPS (ContextMenu)
        0x6F // VK_DIVIDE (numpad / shares scancode with main /)
    )
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroPayload {
    pub menu_key: Option<String>,
    pub menu_mode: Option<String>,
    pub direction_only: Option<bool>,
    pub sequence: Vec<String>,
    pub menu_open_delay: Option<u64>,
    pub press_delay: Option<u64>,
    pub interval_delay: Option<u64>,
}

pub struct MacroRunGuard;

pub fn reserve() -> Result<MacroRunGuard, String> {
    MacroRunGuard::acquire()
}

pub fn execute_reserved(payload: MacroPayload, _guard: MacroRunGuard) -> Result<(), String> {
    if payload.sequence.is_empty() {
        return Ok(());
    }
    let prepared = prepare_macro(&payload)?;
    execute_macro_plan(&prepared.steps)
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedMacro {
    steps: Vec<MacroStep>,
    duration_ms: u64,
}

impl PreparedMacro {
    pub(crate) fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
}

pub(crate) fn prepare_macro(payload: &MacroPayload) -> Result<PreparedMacro, String> {
    let steps = build_macro_plan(payload)?;
    let duration_ms = macro_plan_duration(&steps).ok_or_else(|| {
        format!(
            "Macro duration exceeds the {} second safety limit",
            MAX_MACRO_DURATION_MS / 1000
        )
    })?;
    Ok(PreparedMacro { steps, duration_ms })
}

pub(crate) fn execute_prepared_macro(
    prepared: &PreparedMacro,
    _guard: MacroRunGuard,
) -> Result<(), String> {
    execute_macro_plan(&prepared.steps)
}

impl MacroRunGuard {
    fn acquire() -> Result<Self, String> {
        let (lock, _) = macro_signal();
        let _state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if MACRO_SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err("The application is shutting down".to_owned());
        }
        if MACRO_RUNNING.load(Ordering::Acquire) {
            return Err("Another macro is already running".to_owned());
        }
        MACRO_CANCELLED.store(false, Ordering::Release);
        MACRO_RUNNING.store(true, Ordering::Release);
        Ok(Self)
    }
}

impl Drop for MacroRunGuard {
    fn drop(&mut self) {
        let (lock, changed) = macro_signal();
        let state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        MACRO_RUNNING.store(false, Ordering::Release);
        drop(state);
        changed.notify_all();
    }
}

pub fn cancel_and_wait(timeout: Duration) -> bool {
    let (lock, changed) = macro_signal();
    let state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    MACRO_SHUTTING_DOWN.store(true, Ordering::Release);
    MACRO_CANCELLED.store(true, Ordering::Release);
    changed.notify_all();
    let (state, _) = changed
        .wait_timeout_while(state, timeout, |_| MACRO_RUNNING.load(Ordering::Acquire))
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    drop(state);
    !MACRO_RUNNING.load(Ordering::Acquire)
}

fn macro_signal() -> &'static (Mutex<()>, Condvar) {
    MACRO_SIGNAL.get_or_init(|| (Mutex::new(()), Condvar::new()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MacroStep {
    Down(InputKey),
    Up(InputKey),
    Wait(u64),
}

fn build_macro_plan(payload: &MacroPayload) -> Result<Vec<MacroStep>, String> {
    if payload.sequence.len() > 64 {
        return Err("Macro sequence is too long".to_owned());
    }
    let press_delay = validated_delay(payload.press_delay, 15, "press")?;
    let interval_delay = validated_delay(payload.interval_delay, 15, "interval")?;
    let direction_only = payload.direction_only.unwrap_or(false);

    // The complete plan is validated before the first input is sent, so a bad
    // setting can never leave a partially executed modifier/menu key behind.
    let mut plan = Vec::with_capacity(payload.sequence.len().saturating_mul(4) + 8);
    let menu_state = if direction_only {
        None
    } else {
        let menu_key = match payload.menu_key.as_deref() {
            None => InputKey::Keyboard(VK_LCONTROL),
            Some(name) => input_key(name).ok_or_else(|| format!("Unsupported menu key: {name}"))?,
        };
        let menu_open_delay = validated_delay(payload.menu_open_delay, 150, "menu open")?;
        let hold_mode = match payload.menu_mode.as_deref() {
            None | Some("toggle") => false,
            Some("hold") => true,
            Some(mode) => return Err(format!("Unsupported menu mode: {mode}")),
        };

        plan.extend([MacroStep::Up(menu_key), MacroStep::Wait(10)]);
        if hold_mode {
            plan.push(MacroStep::Down(menu_key));
        } else {
            plan.extend([
                MacroStep::Down(menu_key),
                MacroStep::Wait(press_delay.saturating_add(20)),
                MacroStep::Up(menu_key),
            ]);
        }
        plan.push(MacroStep::Wait(menu_open_delay));
        Some((menu_key, hold_mode))
    };

    for key_name in &payload.sequence {
        let key =
            input_key(key_name).ok_or_else(|| format!("Unsupported input key: {key_name}"))?;
        plan.extend([
            MacroStep::Down(key),
            MacroStep::Wait(press_delay),
            MacroStep::Up(key),
            MacroStep::Wait(interval_delay),
        ]);
    }

    if let Some((menu_key, true)) = menu_state {
        plan.extend([MacroStep::Wait(50), MacroStep::Up(menu_key)]);
    }
    let total_duration = macro_plan_duration(&plan);
    if total_duration.is_none() {
        return Err(format!(
            "Macro duration exceeds the {} second safety limit",
            MAX_MACRO_DURATION_MS / 1000
        ));
    }
    Ok(plan)
}

fn macro_plan_duration(plan: &[MacroStep]) -> Option<u64> {
    let total_duration = plan.iter().try_fold(0_u64, |total, step| match step {
        MacroStep::Wait(delay) => total.checked_add(*delay),
        MacroStep::Down(_) | MacroStep::Up(_) => Some(total),
    });
    total_duration.filter(|duration| *duration <= MAX_MACRO_DURATION_MS)
}

fn execute_macro_plan(plan: &[MacroStep]) -> Result<(), String> {
    let mut held_inputs = HeldInputs::default();
    for step in plan {
        let result = if MACRO_CANCELLED.load(Ordering::Acquire) {
            Err("Macro was cancelled".to_owned())
        } else {
            match *step {
                MacroStep::Down(key) => input_down(key).and_then(|_| held_inputs.track(key)),
                MacroStep::Up(key) => input_up(key).map(|_| {
                    held_inputs.mark_released(key);
                }),
                MacroStep::Wait(milliseconds) => interruptible_sleep(milliseconds),
            }
        };
        if let Err(error) = result {
            // Retry releases in reverse order so modifiers/menu keys do not
            // remain logically held after a partial Windows input failure.
            return match held_inputs.release_all() {
                Some(cleanup) => Err(format!("{error}; input cleanup also failed: {cleanup}")),
                None => Err(error),
            };
        }
    }
    Ok(())
}

#[derive(Default)]
struct HeldInputs {
    keys: [Option<InputKey>; 4],
    len: usize,
}

impl HeldInputs {
    fn track(&mut self, key: InputKey) -> Result<(), String> {
        let Some(slot) = self.keys.get_mut(self.len) else {
            let _ = input_up(key);
            return Err("Too many inputs are held by one macro".to_owned());
        };
        *slot = Some(key);
        self.len += 1;
        Ok(())
    }

    fn mark_released(&mut self, key: InputKey) {
        let Some(index) = (0..self.len).rfind(|index| self.keys[*index] == Some(key)) else {
            return;
        };
        self.keys[index..self.len].rotate_left(1);
        self.len -= 1;
        self.keys[self.len] = None;
    }

    fn release_all(&mut self) -> Option<String> {
        let mut first_error = None;
        while self.len > 0 {
            self.len -= 1;
            let Some(key) = self.keys[self.len].take() else {
                continue;
            };
            if let Err(error) = input_up(key) {
                first_error.get_or_insert(error);
                // SendInput failures can be transient during focus/desktop
                // changes. Make one final best-effort release immediately.
                let _ = input_up(key);
            }
        }
        first_error
    }
}

impl Drop for HeldInputs {
    fn drop(&mut self) {
        let _ = self.release_all();
    }
}

fn normalize_delay(value: Option<u64>, fallback: u64) -> u64 {
    value.filter(|delay| *delay > 0).unwrap_or(fallback)
}

fn validated_delay(value: Option<u64>, fallback: u64, name: &str) -> Result<u64, String> {
    let delay = normalize_delay(value, fallback);
    if delay > MAX_DELAY_MS {
        return Err(format!(
            "Macro {name} delay exceeds the {} second safety limit",
            MAX_DELAY_MS / 1000
        ));
    }
    Ok(delay)
}

fn interruptible_sleep(milliseconds: u64) -> Result<(), String> {
    if MACRO_CANCELLED.load(Ordering::Acquire) {
        return Err("Macro was cancelled".to_owned());
    }
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or_else(|| "Macro delay is unsupported".to_owned())?;
    let (lock, changed) = macro_signal();
    let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if MACRO_CANCELLED.load(Ordering::Acquire) {
            return Err("Macro was cancelled".to_owned());
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(());
        };
        let (next_state, timeout) = changed
            .wait_timeout(state, remaining)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state = next_state;
        if timeout.timed_out() {
            return if MACRO_CANCELLED.load(Ordering::Acquire) {
                Err("Macro was cancelled".to_owned())
            } else {
                Ok(())
            };
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputKey {
    Keyboard(VIRTUAL_KEY),
    ExtendedKeyboard(VIRTUAL_KEY),
    Mouse(MouseButton),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MouseButton {
    Middle,
    Side1,
    Side2,
}

fn input_key(name: &str) -> Option<InputKey> {
    match name {
        "MouseMiddle" => Some(InputKey::Mouse(MouseButton::Middle)),
        "MouseSide1" => Some(InputKey::Mouse(MouseButton::Side1)),
        "MouseSide2" => Some(InputKey::Mouse(MouseButton::Side2)),
        "NumpadEnter" => Some(InputKey::ExtendedKeyboard(VK_RETURN)),
        _ => virtual_key(name).map(InputKey::Keyboard),
    }
}

fn input_down(key: InputKey) -> Result<(), String> {
    match key {
        InputKey::Keyboard(key) => key_down(key),
        InputKey::ExtendedKeyboard(key) => send_key(key, KEYEVENTF_EXTENDEDKEY),
        InputKey::Mouse(button) => send_mouse_button(button, false),
    }
}

fn input_up(key: InputKey) -> Result<(), String> {
    match key {
        InputKey::Keyboard(key) => key_up(key),
        InputKey::ExtendedKeyboard(key) => send_key(key, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
        InputKey::Mouse(button) => send_mouse_button(button, true),
    }
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
    let (virtual_key, scan_code, full_flags) = keyboard_event_components(key, flags);
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: scan_code,
                dwFlags: full_flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // SAFETY: the INPUT discriminant is INPUT_KEYBOARD and the matching `ki`
    // union field is fully initialized. The one-element slice remains valid for
    // the call, and `cbSize` is the exact size Windows requires for INPUT.
    let sent = unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
    if sent == 1 {
        Ok(())
    } else {
        Err(format!(
            "Windows rejected synthetic keyboard input ({sent}/1 events sent). Ensure the game and trigger run at the same privilege level"
        ))
    }
}

fn keyboard_event_components(
    key: VIRTUAL_KEY,
    flags: KEYBD_EVENT_FLAGS,
) -> (VIRTUAL_KEY, u16, KEYBD_EVENT_FLAGS) {
    let vk = key.0;
    let scan_code = scan_code(vk);
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
    let use_scancode = scan_code != 0;
    (
        if use_scancode { VIRTUAL_KEY(0) } else { key },
        scan_code,
        full_flags,
    )
}

fn scan_code(virtual_key: u16) -> u16 {
    // Renderer bindings use KeyboardEvent.code, which names a physical key.
    // Fixed Set-1 scan codes preserve that contract on QWERTY, AZERTY and
    // QWERTZ layouts and avoid a Win32 layout lookup on every input edge.
    const LETTERS: [u16; 26] = [
        0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18,
        0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c,
    ];
    const DIGITS: [u16; 10] = [0x0b, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a];
    const NUMPAD_DIGITS: [u16; 10] = [0x52, 0x4f, 0x50, 0x51, 0x4b, 0x4c, 0x4d, 0x47, 0x48, 0x49];

    match virtual_key {
        0x41..=0x5a => LETTERS[usize::from(virtual_key - 0x41)],
        0x30..=0x39 => DIGITS[usize::from(virtual_key - 0x30)],
        0x60..=0x69 => NUMPAD_DIGITS[usize::from(virtual_key - 0x60)],
        0x70..=0x79 => 0x3b + (virtual_key - 0x70),
        0x7a => 0x57,
        0x7b => 0x58,
        0x7c..=0x86 => 0x64 + (virtual_key - 0x7c),
        0x87 => 0x76,
        0x08 => 0x0e,        // Backspace
        0x09 => 0x0f,        // Tab
        0x0d => 0x1c,        // Enter / NumpadEnter
        0x14 => 0x3a,        // CapsLock
        0x1b => 0x01,        // Escape
        0x20 => 0x39,        // Space
        0x21 => 0x49,        // PageUp
        0x22 => 0x51,        // PageDown
        0x23 => 0x4f,        // End
        0x24 => 0x47,        // Home
        0x25 => 0x4b,        // ArrowLeft
        0x26 => 0x48,        // ArrowUp
        0x27 => 0x4d,        // ArrowRight
        0x28 => 0x50,        // ArrowDown
        0x2d => 0x52,        // Insert
        0x2e => 0x53,        // Delete
        0x5b => 0x5b,        // MetaLeft
        0x5c => 0x5c,        // MetaRight
        0x5d => 0x5d,        // ContextMenu
        0x6a => 0x37,        // NumpadMultiply
        0x6b => 0x4e,        // NumpadAdd
        0x6d => 0x4a,        // NumpadSubtract
        0x6e => 0x53,        // NumpadDecimal
        0x6f => 0x35,        // NumpadDivide
        0xa0 => 0x2a,        // ShiftLeft
        0xa1 => 0x36,        // ShiftRight
        0xa2 | 0xa3 => 0x1d, // ControlLeft / ControlRight
        0xa4 | 0xa5 => 0x38, // AltLeft / AltRight
        0xba => 0x27,        // Semicolon
        0xbb => 0x0d,        // Equal
        0xbc => 0x33,        // Comma
        0xbd => 0x0c,        // Minus
        0xbe => 0x34,        // Period
        0xbf => 0x35,        // Slash
        0xc0 => 0x29,        // Backquote
        0xdb => 0x1a,        // BracketLeft
        0xdc => 0x2b,        // Backslash
        0xdd => 0x1b,        // BracketRight
        0xde => 0x28,        // Quote
        _ => 0,
    }
}

fn send_mouse_button(button: MouseButton, release: bool) -> Result<(), String> {
    let (flags, mouse_data) = mouse_event(button, release);
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // SAFETY: the INPUT discriminant is INPUT_MOUSE and the matching `mi` union
    // field is fully initialized. The one-element slice remains valid for the
    // call, and `cbSize` is the exact size Windows requires for INPUT.
    let sent = unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
    if sent == 1 {
        Ok(())
    } else {
        Err(format!(
            "Windows rejected synthetic mouse input ({sent}/1 events sent). Ensure the game and trigger run at the same privilege level"
        ))
    }
}

fn mouse_event(button: MouseButton, release: bool) -> (MOUSE_EVENT_FLAGS, u32) {
    match button {
        MouseButton::Middle => (
            if release {
                MOUSEEVENTF_MIDDLEUP
            } else {
                MOUSEEVENTF_MIDDLEDOWN
            },
            0,
        ),
        MouseButton::Side1 => (
            if release {
                MOUSEEVENTF_XUP
            } else {
                MOUSEEVENTF_XDOWN
            },
            1,
        ),
        MouseButton::Side2 => (
            if release {
                MOUSEEVENTF_XUP
            } else {
                MOUSEEVENTF_XDOWN
            },
            2,
        ),
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

pub(crate) fn filter_virtual_key(name: &str) -> Option<u16> {
    virtual_key(name).map(|key| key.0)
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
    if value.len() != 1 {
        return None;
    }
    let byte = value.as_bytes().first().copied()?.to_ascii_uppercase();
    (byte.is_ascii_uppercase()).then_some(VIRTUAL_KEY(byte as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MacroShutdownReset;

    impl Drop for MacroShutdownReset {
        fn drop(&mut self) {
            let (lock, changed) = macro_signal();
            let state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            MACRO_RUNNING.store(false, Ordering::Release);
            MACRO_CANCELLED.store(false, Ordering::Release);
            MACRO_SHUTTING_DOWN.store(false, Ordering::Release);
            drop(state);
            changed.notify_all();
        }
    }

    #[test]
    fn maps_the_existing_renderer_key_names() {
        assert_eq!(virtual_key("KeyW"), Some(VIRTUAL_KEY(b'W' as u16)));
        assert_eq!(virtual_key("Digit4"), Some(VIRTUAL_KEY(b'4' as u16)));
        assert_eq!(virtual_key("F8"), Some(VIRTUAL_KEY(VK_F1.0 + 7)));
        assert_eq!(virtual_key("Numpad6"), Some(VIRTUAL_KEY(VK_NUMPAD0.0 + 6)));
        assert_eq!(virtual_key("NotAKey"), None);
        assert_eq!(virtual_key("KeyAB"), None);
    }

    #[test]
    fn shutdown_wakes_macro_waits_and_rejects_late_reservations() {
        let _reset = MacroShutdownReset;
        let guard = reserve().expect("macro slot should be available");
        let worker = std::thread::spawn(move || {
            let result = interruptible_sleep(MAX_DELAY_MS);
            drop(guard);
            result
        });
        let started = Instant::now();
        assert!(cancel_and_wait(Duration::from_millis(250)));
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(worker
            .join()
            .expect("macro worker should not panic")
            .is_err());
        assert!(reserve().is_err());
    }

    #[test]
    fn maps_and_builds_mouse_button_inputs() {
        assert_eq!(
            input_key("MouseSide1"),
            Some(InputKey::Mouse(MouseButton::Side1))
        );
        assert_eq!(
            input_key("MouseSide2"),
            Some(InputKey::Mouse(MouseButton::Side2))
        );
        assert_eq!(
            mouse_event(MouseButton::Side1, false),
            (MOUSEEVENTF_XDOWN, 1)
        );
        assert_eq!(mouse_event(MouseButton::Side2, true), (MOUSEEVENTF_XUP, 2));
        assert_eq!(
            input_key("NumpadEnter"),
            Some(InputKey::ExtendedKeyboard(VK_RETURN))
        );
    }

    #[test]
    fn sends_arrow_keys_as_extended_scancodes() {
        let (virtual_key, scan_code, flags) =
            keyboard_event_components(VK_UP, KEYBD_EVENT_FLAGS(0));
        assert_eq!(virtual_key, VIRTUAL_KEY(0));
        assert_ne!(scan_code, 0);
        assert_ne!(flags.0 & KEYEVENTF_SCANCODE.0, 0);
        assert_ne!(flags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);

        let (_, _, alt_flags) = keyboard_event_components(VK_LMENU, KEYBD_EVENT_FLAGS(0));
        assert_ne!(alt_flags.0 & KEYEVENTF_SCANCODE.0, 0);
        assert_eq!(alt_flags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);

        let (_, _, context_flags) = keyboard_event_components(VK_APPS, KEYBD_EVENT_FLAGS(0));
        assert_ne!(context_flags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);
    }

    #[test]
    fn uses_layout_independent_physical_scan_codes() {
        assert_eq!(scan_code(b'W' as u16), 0x11);
        assert_eq!(scan_code(b'A' as u16), 0x1e);
        assert_eq!(scan_code(b'1' as u16), 0x02);

        let (_, arrow_up_scan, arrow_up_flags) =
            keyboard_event_components(VK_UP, KEYBD_EVENT_FLAGS(0));
        let (_, numpad_eight_scan, numpad_eight_flags) =
            keyboard_event_components(VIRTUAL_KEY(0x68), KEYBD_EVENT_FLAGS(0));
        assert_eq!(arrow_up_scan, numpad_eight_scan);
        assert_ne!(arrow_up_flags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);
        assert_eq!(numpad_eight_flags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);

        let (_, enter_scan, enter_flags) =
            keyboard_event_components(VK_RETURN, KEYBD_EVENT_FLAGS(0));
        let (_, numpad_enter_scan, numpad_enter_flags) =
            keyboard_event_components(VK_RETURN, KEYEVENTF_EXTENDEDKEY);
        assert_eq!(enter_scan, numpad_enter_scan);
        assert_eq!(enter_flags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);
        assert_ne!(numpad_enter_flags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);

        let (_, divide_scan, divide_flags) =
            keyboard_event_components(VK_DIVIDE, KEYBD_EVENT_FLAGS(0));
        assert_eq!(divide_scan, 0x35);
        assert_ne!(divide_flags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);
    }

    #[test]
    fn keeps_the_electron_delay_defaults() {
        assert_eq!(normalize_delay(None, 150), 150);
        assert_eq!(normalize_delay(Some(0), 15), 15);
        assert_eq!(normalize_delay(Some(7), 15), 7);
    }

    #[test]
    fn orders_menu_before_configured_direction_keys_in_toggle_mode() {
        let payload = MacroPayload {
            menu_key: Some("AltLeft".to_owned()),
            menu_mode: Some("toggle".to_owned()),
            direction_only: None,
            sequence: vec!["ArrowUp".to_owned(), "ArrowRight".to_owned()],
            menu_open_delay: Some(15),
            press_delay: Some(20),
            interval_delay: Some(15),
        };

        assert_eq!(
            build_macro_plan(&payload).expect("valid settings should build a macro plan"),
            vec![
                MacroStep::Up(InputKey::Keyboard(VK_LMENU)),
                MacroStep::Wait(10),
                MacroStep::Down(InputKey::Keyboard(VK_LMENU)),
                MacroStep::Wait(40),
                MacroStep::Up(InputKey::Keyboard(VK_LMENU)),
                MacroStep::Wait(15),
                MacroStep::Down(InputKey::Keyboard(VK_UP)),
                MacroStep::Wait(20),
                MacroStep::Up(InputKey::Keyboard(VK_UP)),
                MacroStep::Wait(15),
                MacroStep::Down(InputKey::Keyboard(VK_RIGHT)),
                MacroStep::Wait(20),
                MacroStep::Up(InputKey::Keyboard(VK_RIGHT)),
                MacroStep::Wait(15),
            ]
        );
    }

    #[test]
    fn keeps_menu_held_until_every_direction_is_released() {
        let payload = MacroPayload {
            menu_key: Some("ControlLeft".to_owned()),
            menu_mode: Some("hold".to_owned()),
            direction_only: None,
            sequence: vec!["W".to_owned()],
            menu_open_delay: Some(100),
            press_delay: Some(10),
            interval_delay: Some(15),
        };
        let menu_key = InputKey::Keyboard(VK_LCONTROL);
        let direction_key = InputKey::Keyboard(VIRTUAL_KEY(b'W' as u16));

        assert_eq!(
            build_macro_plan(&payload).expect("valid settings should build a macro plan"),
            vec![
                MacroStep::Up(menu_key),
                MacroStep::Wait(10),
                MacroStep::Down(menu_key),
                MacroStep::Wait(100),
                MacroStep::Down(direction_key),
                MacroStep::Wait(10),
                MacroStep::Up(direction_key),
                MacroStep::Wait(15),
                MacroStep::Wait(50),
                MacroStep::Up(menu_key),
            ]
        );
    }

    #[test]
    fn direction_only_never_touches_or_waits_for_the_menu_key() {
        let payload = MacroPayload {
            // Direction-only mode deliberately ignores stale/invalid menu
            // settings because none of those inputs will be emitted.
            menu_key: Some("NotAKey".to_owned()),
            menu_mode: Some("invalid".to_owned()),
            direction_only: Some(true),
            sequence: vec!["W".to_owned(), "D".to_owned()],
            menu_open_delay: Some(MAX_DELAY_MS + 1),
            press_delay: Some(10),
            interval_delay: Some(15),
        };

        assert_eq!(
            build_macro_plan(&payload).expect("menu settings are unused in direction-only mode"),
            vec![
                MacroStep::Down(InputKey::Keyboard(VIRTUAL_KEY(b'W' as u16))),
                MacroStep::Wait(10),
                MacroStep::Up(InputKey::Keyboard(VIRTUAL_KEY(b'W' as u16))),
                MacroStep::Wait(15),
                MacroStep::Down(InputKey::Keyboard(VIRTUAL_KEY(b'D' as u16))),
                MacroStep::Wait(10),
                MacroStep::Up(InputKey::Keyboard(VIRTUAL_KEY(b'D' as u16))),
                MacroStep::Wait(15),
            ]
        );
    }

    #[test]
    fn rejects_unbounded_or_malformed_macro_settings_before_execution() {
        let base = MacroPayload {
            menu_key: Some("ControlLeft".to_owned()),
            menu_mode: Some("hold".to_owned()),
            direction_only: None,
            sequence: vec!["W".to_owned()],
            menu_open_delay: Some(100),
            press_delay: Some(10),
            interval_delay: Some(15),
        };

        let mut payload = base.clone();
        payload.menu_open_delay = Some(MAX_DELAY_MS + 1);
        assert!(build_macro_plan(&payload).is_err());

        let mut payload = base.clone();
        payload.menu_mode = Some("invalid".to_owned());
        assert!(build_macro_plan(&payload).is_err());

        let mut payload = base.clone();
        payload.menu_key = Some("NotAKey".to_owned());
        assert!(build_macro_plan(&payload).is_err());

        let mut payload = base;
        payload.sequence = vec!["W".to_owned(); 64];
        payload.press_delay = Some(3_000);
        payload.interval_delay = Some(3_000);
        assert!(build_macro_plan(&payload).is_err());
    }

    #[test]
    fn fixed_held_input_tracker_removes_the_most_recent_matching_key() {
        let first = InputKey::Keyboard(VIRTUAL_KEY(b'W' as u16));
        let second = InputKey::Keyboard(VIRTUAL_KEY(b'A' as u16));
        let mut held = HeldInputs {
            keys: [Some(first), Some(second), Some(first), None],
            len: 3,
        };
        held.mark_released(first);
        assert_eq!(held.keys, [Some(first), Some(second), None, None]);
        held.len = 0;
        held.keys = [None; 4];
    }
}
