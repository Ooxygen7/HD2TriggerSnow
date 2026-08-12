use crate::{input, runtime_diagnostics};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        mpsc, Arc, Mutex, OnceLock, RwLock,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::GetAsyncKeyState,
        WindowsAndMessaging::{
            CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
            UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL,
            WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MBUTTONUP,
            WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN,
            WM_XBUTTONUP,
        },
    },
};

static LISTENER_STARTED: OnceLock<()> = OnceLock::new();
static EVENT_SENDER: OnceLock<mpsc::SyncSender<InputEvent>> = OnceLock::new();
static SHORTCUT_STATE: OnceLock<RwLock<ShortcutState>> = OnceLock::new();
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static DROPPED_EVENTS: AtomicU64 = AtomicU64::new(0);
static QUEUED_EVENTS: AtomicU64 = AtomicU64::new(0);
static PROCESSED_EVENTS: AtomicU64 = AtomicU64::new(0);
static QUEUE_DEPTH: AtomicU64 = AtomicU64::new(0);
static MAX_QUEUE_DEPTH: AtomicU64 = AtomicU64::new(0);
static STATE_RECONCILIATIONS: AtomicU64 = AtomicU64::new(0);
static STALE_EDGE_RECOVERIES: AtomicU64 = AtomicU64::new(0);
static ASYNC_STATE_CORRECTIONS: AtomicU64 = AtomicU64::new(0);
static NATIVE_MACROS_STARTED: AtomicU64 = AtomicU64::new(0);
static NATIVE_MACROS_SUPPRESSED: AtomicU64 = AtomicU64::new(0);
static NATIVE_ACTIONS_ROUTED: AtomicU64 = AtomicU64::new(0);
static BINDING_EVENTS_FORWARDED: AtomicU64 = AtomicU64::new(0);
static OWN_SYNTHETIC_EVENTS_IGNORED: AtomicU64 = AtomicU64::new(0);
static EXTERNAL_INJECTED_EVENTS_ACCEPTED: AtomicU64 = AtomicU64::new(0);
static RELEVANT_INPUT_EDGES: AtomicU64 = AtomicU64::new(0);
static SHORTCUTS_MATCHED: AtomicU64 = AtomicU64::new(0);
static UNMATCHED_SHORTCUT_EDGES: AtomicU64 = AtomicU64::new(0);
static NATIVE_MACROS_COMPLETED: AtomicU64 = AtomicU64::new(0);
static NATIVE_MACROS_FAILED: AtomicU64 = AtomicU64::new(0);
static LAST_RELEVANT_INPUT_AT_UNIX_MS: AtomicU64 = AtomicU64::new(0);
static LAST_SHORTCUT_MATCH_AT_UNIX_MS: AtomicU64 = AtomicU64::new(0);
static LAST_MACRO_COMPLETION_AT_UNIX_MS: AtomicU64 = AtomicU64::new(0);
static FILTER_INITIALIZED: AtomicBool = AtomicBool::new(false);
static CAPTURE_ALL_INPUTS: AtomicBool = AtomicBool::new(true);
static KEY_FILTER: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static CALIBRATION_KEY_FILTER: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static PRESSED_KEYS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static LAST_KEYDOWN_TIME: [AtomicU32; 256] = [const { AtomicU32::new(0) }; 256];
static PRESSED_MOUSE: AtomicU8 = AtomicU8::new(0);
static LAST_MOUSE_DOWN_TIME: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
static MOUSE_FILTER: AtomicU8 = AtomicU8::new(0);
static CALIBRATION_MOUSE_FILTER: AtomicU8 = AtomicU8::new(0);
static WHEEL_FILTER: AtomicBool = AtomicBool::new(false);
static FILTER_UPDATE_LOCK: Mutex<()> = Mutex::new(());

const LLKHF_INJECTED_FLAG: u32 = 0x10;
const LLMHF_INJECTED_FLAG: u32 = 0x01;
const FILTER_MOUSE_MIDDLE: u8 = 1 << 0;
const FILTER_MOUSE_SIDE1: u8 = 1 << 1;
const FILTER_MOUSE_SIDE2: u8 = 1 << 2;
const PRESSED_NUMPAD_ENTER: u32 = 0xff;
const EVENT_QUEUE_CAPACITY: usize = 1024;
const STALE_PRESSED_EDGE_MS: u32 = 5_000;
const MAX_NATIVE_MACROS: usize = 64;
const MAX_CHORD_KEYS: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

impl PressedInputs {
    fn contains(self, required: Self) -> bool {
        self.keys
            .into_iter()
            .zip(required.keys)
            .all(|(held, expected)| held & expected == expected)
            && self.mouse & required.mouse == required.mouse
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GlobalInputPayload {
    key: &'static str,
    pressed_inputs: Vec<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputSource {
    Keyboard,
    Mouse,
    Wheel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InputEdge {
    source: InputSource,
    key: &'static str,
    pressed: PressedInputs,
    captured_for_binding: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputEvent {
    Edge(InputEdge),
    Shutdown,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutConfig {
    #[serde(default)]
    macros: Vec<MacroShortcutConfig>,
    ocr_hotkey: Option<String>,
    overlay_visible: bool,
    overlay_up: Option<String>,
    overlay_down: Option<String>,
    overlay_exec: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MacroShortcutConfig {
    hotkey: String,
    payload: input::MacroPayload,
    overlay_index: i64,
}

#[derive(Clone, Debug)]
struct ShortcutChord {
    trigger: &'static str,
    required: PressedInputs,
    key_count: usize,
}

impl ShortcutChord {
    fn matches(&self, key: &str, pressed: PressedInputs) -> bool {
        self.trigger == key && pressed.contains(self.required)
    }
}

#[derive(Clone, Debug)]
struct NativeMacro {
    chord: ShortcutChord,
    prepared: Arc<input::PreparedMacro>,
    overlay_index: i64,
}

#[derive(Clone, Debug, Default)]
struct ShortcutState {
    macros: Vec<NativeMacro>,
    ocr_hotkey: Option<&'static str>,
    overlay_up: Option<&'static str>,
    overlay_down: Option<&'static str>,
    overlay_exec: Option<&'static str>,
    calibration: InputFilter,
    triggers: InputFilter,
}

#[derive(Clone, Debug)]
enum RoutedShortcut {
    Macro(NativeMacro),
    Action(&'static str),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeMacroEvent {
    overlay_index: i64,
    duration: u64,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct NativeShortcutEvent {
    action: &'static str,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputDiagnostics {
    hook_running: bool,
    filter_initialized: bool,
    capture_all_inputs: bool,
    queue_capacity: u64,
    queued_events: u64,
    processed_events: u64,
    dropped_events: u64,
    queue_depth: u64,
    max_queue_depth: u64,
    state_reconciliations: u64,
    stale_edge_recoveries: u64,
    async_state_corrections: u64,
    native_macro_bindings: u64,
    native_macros_started: u64,
    native_macros_suppressed: u64,
    native_actions_routed: u64,
    binding_events_forwarded: u64,
    own_synthetic_events_ignored: u64,
    external_injected_events_accepted: u64,
    relevant_input_edges: u64,
    shortcuts_matched: u64,
    unmatched_shortcut_edges: u64,
    native_macros_completed: u64,
    native_macros_failed: u64,
    last_relevant_input_at_unix_ms: Option<u64>,
    last_shortcut_match_at_unix_ms: Option<u64>,
    last_macro_completion_at_unix_ms: Option<u64>,
}

pub fn start(app: AppHandle) -> Result<(), String> {
    if LISTENER_STARTED.get().is_some() {
        return Ok(());
    }
    reset_pressed_inputs();
    let (event_sender, event_receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
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
        // The sender is bounded, so a best-effort try_send could lose the only
        // shutdown marker when the queue is full and leave the worker blocked
        // forever. The worker drains fixed-size events quickly; a blocking send
        // guarantees orderly termination and provides natural backpressure.
        let _ = sender.send(InputEvent::Shutdown);
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

pub fn configure(config: ShortcutConfig, capture_all: bool) -> Result<(), String> {
    let state = ShortcutState::build(config)?;
    let trigger_filter = state.triggers;
    let calibration_filter = state.calibration;
    let _writer = FILTER_UPDATE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    // Forward everything while the multi-atomic snapshot is replaced so an
    // edge can never be matched against a half-old, half-new shortcut table.
    // The frontend ignores these transitional raw events unless it is actively
    // recording a binding. The writer lock prevents overlapping snapshots.
    let was_capturing_all = forwards_all_inputs();
    CAPTURE_ALL_INPUTS.store(true, Ordering::Release);
    *shortcut_state()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = state;
    for (destination, value) in KEY_FILTER.iter().zip(trigger_filter.keys) {
        destination.store(value, Ordering::Release);
    }
    for (destination, value) in CALIBRATION_KEY_FILTER.iter().zip(calibration_filter.keys) {
        destination.store(value, Ordering::Release);
    }
    MOUSE_FILTER.store(trigger_filter.mouse, Ordering::Release);
    CALIBRATION_MOUSE_FILTER.store(calibration_filter.mouse, Ordering::Release);
    WHEEL_FILTER.store(trigger_filter.wheel, Ordering::Release);
    FILTER_INITIALIZED.store(true, Ordering::Release);
    if capture_all && !was_capturing_all {
        // Main Enter and NumpadEnter share VK_RETURN, so GetAsyncKeyState cannot
        // reconstruct which one was held before binding started. Clear the
        // synthetic NumpadEnter bit and require a fresh edge instead of adding a
        // stale key to a newly recorded chord.
        mark_key_released(PRESSED_NUMPAD_ENTER);
    }
    CAPTURE_ALL_INPUTS.store(capture_all, Ordering::Release);
    Ok(())
}

pub fn diagnostics() -> InputDiagnostics {
    let native_macro_bindings = shortcut_state()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .macros
        .len() as u64;
    InputDiagnostics {
        hook_running: HOOK_THREAD_ID.load(Ordering::Acquire) != 0,
        filter_initialized: FILTER_INITIALIZED.load(Ordering::Acquire),
        capture_all_inputs: forwards_all_inputs(),
        queue_capacity: EVENT_QUEUE_CAPACITY as u64,
        queued_events: QUEUED_EVENTS.load(Ordering::Relaxed),
        processed_events: PROCESSED_EVENTS.load(Ordering::Relaxed),
        dropped_events: DROPPED_EVENTS.load(Ordering::Relaxed),
        queue_depth: QUEUE_DEPTH.load(Ordering::Relaxed),
        max_queue_depth: MAX_QUEUE_DEPTH.load(Ordering::Relaxed),
        state_reconciliations: STATE_RECONCILIATIONS.load(Ordering::Relaxed),
        stale_edge_recoveries: STALE_EDGE_RECOVERIES.load(Ordering::Relaxed),
        async_state_corrections: ASYNC_STATE_CORRECTIONS.load(Ordering::Relaxed),
        native_macro_bindings,
        native_macros_started: NATIVE_MACROS_STARTED.load(Ordering::Relaxed),
        native_macros_suppressed: NATIVE_MACROS_SUPPRESSED.load(Ordering::Relaxed),
        native_actions_routed: NATIVE_ACTIONS_ROUTED.load(Ordering::Relaxed),
        binding_events_forwarded: BINDING_EVENTS_FORWARDED.load(Ordering::Relaxed),
        own_synthetic_events_ignored: OWN_SYNTHETIC_EVENTS_IGNORED.load(Ordering::Relaxed),
        external_injected_events_accepted: EXTERNAL_INJECTED_EVENTS_ACCEPTED
            .load(Ordering::Relaxed),
        relevant_input_edges: RELEVANT_INPUT_EDGES.load(Ordering::Relaxed),
        shortcuts_matched: SHORTCUTS_MATCHED.load(Ordering::Relaxed),
        unmatched_shortcut_edges: UNMATCHED_SHORTCUT_EDGES.load(Ordering::Relaxed),
        native_macros_completed: NATIVE_MACROS_COMPLETED.load(Ordering::Relaxed),
        native_macros_failed: NATIVE_MACROS_FAILED.load(Ordering::Relaxed),
        last_relevant_input_at_unix_ms: nonzero_timestamp(
            LAST_RELEVANT_INPUT_AT_UNIX_MS.load(Ordering::Relaxed),
        ),
        last_shortcut_match_at_unix_ms: nonzero_timestamp(
            LAST_SHORTCUT_MATCH_AT_UNIX_MS.load(Ordering::Relaxed),
        ),
        last_macro_completion_at_unix_ms: nonzero_timestamp(
            LAST_MACRO_COMPLETION_AT_UNIX_MS.load(Ordering::Relaxed),
        ),
    }
}

fn nonzero_timestamp(timestamp: u64) -> Option<u64> {
    (timestamp != 0).then_some(timestamp)
}

fn shortcut_state() -> &'static RwLock<ShortcutState> {
    SHORTCUT_STATE.get_or_init(|| RwLock::new(ShortcutState::default()))
}

impl ShortcutState {
    fn build(config: ShortcutConfig) -> Result<Self, String> {
        if config.macros.len() > MAX_NATIVE_MACROS {
            return Err(format!(
                "Native shortcut table exceeds the {MAX_NATIVE_MACROS} macro safety limit"
            ));
        }

        let mut state = Self::default();
        state
            .macros
            .try_reserve_exact(config.macros.len())
            .map_err(|_| "Cannot allocate the native shortcut table".to_owned())?;
        for binding in config.macros {
            let (chord, calibration) = parse_shortcut_chord(&binding.hotkey)?;
            insert_filter_name(&mut state.triggers, chord.trigger)?;
            state.calibration.merge(calibration);
            let prepared = Arc::new(input::prepare_macro(&binding.payload)?);
            state.macros.push(NativeMacro {
                chord,
                prepared,
                overlay_index: binding.overlay_index,
            });
        }

        state.ocr_hotkey = parse_optional_shortcut(config.ocr_hotkey, "OCR")?;
        if config.overlay_visible {
            state.overlay_exec = parse_optional_shortcut(config.overlay_exec, "overlay execute")?;
            state.overlay_up = parse_optional_shortcut(config.overlay_up, "overlay up")?;
            state.overlay_down = parse_optional_shortcut(config.overlay_down, "overlay down")?;
        }
        for key in [
            state.ocr_hotkey,
            state.overlay_exec,
            state.overlay_up,
            state.overlay_down,
        ]
        .into_iter()
        .flatten()
        {
            insert_filter_name(&mut state.triggers, key)?;
        }
        Ok(state)
    }

    fn route(&self, key: &str, pressed: PressedInputs) -> Option<RoutedShortcut> {
        let mut best_chord: Option<&NativeMacro> = None;
        for binding in &self.macros {
            if binding.chord.key_count < 2 || !binding.chord.matches(key, pressed) {
                continue;
            }
            if best_chord.is_none_or(|current| binding.chord.key_count > current.chord.key_count) {
                best_chord = Some(binding);
            }
        }
        if let Some(binding) = best_chord {
            return Some(RoutedShortcut::Macro(binding.clone()));
        }

        if self.ocr_hotkey == Some(key) {
            return Some(RoutedShortcut::Action("ocr"));
        }
        if self.overlay_exec == Some(key) {
            return Some(RoutedShortcut::Action("overlay-exec"));
        }
        if self.overlay_up == Some(key) {
            return Some(RoutedShortcut::Action("overlay-up"));
        }
        if self.overlay_down == Some(key) {
            return Some(RoutedShortcut::Action("overlay-down"));
        }
        self.macros
            .iter()
            .find(|binding| binding.chord.matches(key, pressed))
            .cloned()
            .map(RoutedShortcut::Macro)
    }
}

impl InputFilter {
    fn merge(&mut self, other: Self) {
        for (destination, value) in self.keys.iter_mut().zip(other.keys) {
            *destination |= value;
        }
        self.mouse |= other.mouse;
        self.wheel |= other.wheel;
    }
}

fn parse_optional_shortcut(
    value: Option<String>,
    label: &str,
) -> Result<Option<&'static str>, String> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    canonical_key_name(&value)
        .map(Some)
        .ok_or_else(|| format!("Unsupported {label} shortcut: {value}"))
}

fn parse_shortcut_chord(value: &str) -> Result<(ShortcutChord, InputFilter), String> {
    let parts = value.split('+').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > MAX_CHORD_KEYS || parts.iter().any(|part| part.is_empty())
    {
        return Err(format!(
            "Shortcut chord must contain between 1 and {MAX_CHORD_KEYS} keys"
        ));
    }

    let mut canonical = Vec::with_capacity(parts.len());
    for part in parts {
        let key =
            canonical_key_name(part).ok_or_else(|| format!("Unsupported shortcut key: {part}"))?;
        if canonical.contains(&key) {
            return Err(format!("Shortcut chord contains duplicate key: {key}"));
        }
        canonical.push(key);
    }
    let trigger = *canonical
        .last()
        .ok_or_else(|| "Shortcut chord is empty".to_owned())?;
    let mut required = PressedInputs::default();
    let mut calibration = InputFilter::default();
    for key in canonical.iter().take(canonical.len() - 1) {
        add_required_input(&mut required, &mut calibration, key)?;
    }
    Ok((
        ShortcutChord {
            trigger,
            required,
            key_count: canonical.len(),
        },
        calibration,
    ))
}

fn add_required_input(
    required: &mut PressedInputs,
    calibration: &mut InputFilter,
    key: &str,
) -> Result<(), String> {
    match key {
        "MouseMiddle" => {
            required.mouse |= FILTER_MOUSE_MIDDLE;
            calibration.mouse |= FILTER_MOUSE_MIDDLE;
        }
        "MouseSide1" => {
            required.mouse |= FILTER_MOUSE_SIDE1;
            calibration.mouse |= FILTER_MOUSE_SIDE1;
        }
        "MouseSide2" => {
            required.mouse |= FILTER_MOUSE_SIDE2;
            calibration.mouse |= FILTER_MOUSE_SIDE2;
        }
        "WheelUp" | "WheelDown" => {
            return Err("A mouse wheel edge can only be the final chord trigger".to_owned());
        }
        name => {
            let virtual_key = input::filter_virtual_key(name)
                .ok_or_else(|| format!("Unsupported shortcut requirement: {name}"))?;
            let state_code = pressed_state_code(name, u32::from(virtual_key));
            insert_pressed_key(&mut required.keys, state_code)?;
            // GetAsyncKeyState exposes one VK_RETURN state for both physical
            // Enter keys. Calibrating either from it could make an Enter chord
            // match NumpadEnter (or vice versa), so both rely on exact hook
            // edges and the stale-edge recovery below.
            if name != "Enter" && name != "NumpadEnter" {
                insert_filtered_key(&mut calibration.keys, virtual_key);
            }
        }
    }
    Ok(())
}

fn insert_pressed_key(keys: &mut [u64; 4], code: u32) -> Result<(), String> {
    let code = usize::try_from(code).map_err(|_| "Shortcut key code is invalid".to_owned())?;
    let word = keys
        .get_mut(code / 64)
        .ok_or_else(|| "Shortcut key code is out of range".to_owned())?;
    *word |= 1_u64 << (code % 64);
    Ok(())
}

fn canonical_key_name(value: &str) -> Option<&'static str> {
    match value {
        "MouseMiddle" => Some("MouseMiddle"),
        "MouseSide1" => Some("MouseSide1"),
        "MouseSide2" => Some("MouseSide2"),
        "WheelUp" => Some("WheelUp"),
        "WheelDown" => Some("WheelDown"),
        "NumpadEnter" => Some("NumpadEnter"),
        name => input::filter_virtual_key(name)
            .and_then(|virtual_key| key_name(u32::from(virtual_key), 0, 0)),
    }
}

#[cfg(test)]
fn build_filter(keys: &[String]) -> Result<InputFilter, String> {
    let mut filter = InputFilter::default();
    for key in keys {
        insert_filter_name(&mut filter, key)?;
    }
    Ok(filter)
}

fn insert_filter_name(filter: &mut InputFilter, name: &str) -> Result<(), String> {
    match name {
        "MouseMiddle" => filter.mouse |= FILTER_MOUSE_MIDDLE,
        "MouseSide1" => filter.mouse |= FILTER_MOUSE_SIDE1,
        "MouseSide2" => filter.mouse |= FILTER_MOUSE_SIDE2,
        "WheelUp" | "WheelDown" => filter.wheel = true,
        name => {
            let code = input::filter_virtual_key(name)
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
    Ok(())
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
    for timestamp in &LAST_KEYDOWN_TIME {
        timestamp.store(0, Ordering::Release);
    }
    PRESSED_MOUSE.store(0, Ordering::Release);
    for timestamp in &LAST_MOUSE_DOWN_TIME {
        timestamp.store(0, Ordering::Release);
    }
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

fn key_is_marked_pressed(virtual_key: u32) -> bool {
    let Ok(virtual_key) = usize::try_from(virtual_key) else {
        return false;
    };
    PRESSED_KEYS
        .get(virtual_key / 64)
        .is_some_and(|word| word.load(Ordering::Acquire) & (1_u64 << (virtual_key % 64)) != 0)
}

fn reconcile_current_key_before_down(virtual_key: u32, physically_pressed: bool) {
    if !physically_pressed && key_is_marked_pressed(virtual_key) {
        reconcile_key_state(virtual_key, false);
    }
}

fn mark_key_pressed_at(virtual_key: u32, event_time: u32) -> bool {
    let is_new = mark_key_pressed(virtual_key);
    let previous_time = usize::try_from(virtual_key)
        .ok()
        .and_then(|index| LAST_KEYDOWN_TIME.get(index))
        .map(|timestamp| timestamp.swap(event_time, Ordering::AcqRel))
        .unwrap_or(0);
    if is_new {
        return true;
    }
    if repeated_edge_is_recovered(previous_time, event_time) {
        STATE_RECONCILIATIONS.fetch_add(1, Ordering::Relaxed);
        STALE_EDGE_RECOVERIES.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    false
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

fn reconcile_current_mouse_before_down(mask: u8, physically_pressed: bool) {
    if !physically_pressed && PRESSED_MOUSE.load(Ordering::Acquire) & mask != 0 {
        reconcile_mouse_state(mask, false);
    }
}

fn mark_mouse_pressed_at(mask: u8, event_time: u32) -> bool {
    let is_new = mark_mouse_pressed(mask);
    let previous_time = mouse_filter_index(mask)
        .and_then(|index| LAST_MOUSE_DOWN_TIME.get(index))
        .map(|timestamp| timestamp.swap(event_time, Ordering::AcqRel))
        .unwrap_or(0);
    if is_new {
        return true;
    }
    if repeated_edge_is_recovered(previous_time, event_time) {
        STATE_RECONCILIATIONS.fetch_add(1, Ordering::Relaxed);
        STALE_EDGE_RECOVERIES.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    false
}

fn repeated_edge_is_recovered(previous_time: u32, event_time: u32) -> bool {
    previous_time == 0 || event_time.wrapping_sub(previous_time) >= STALE_PRESSED_EDGE_MS
}

fn mark_mouse_released(mask: u8) {
    PRESSED_MOUSE.fetch_and(!mask, Ordering::AcqRel);
}

fn mouse_filter_index(mask: u8) -> Option<usize> {
    match mask {
        FILTER_MOUSE_MIDDLE => Some(0),
        FILTER_MOUSE_SIDE1 => Some(1),
        FILTER_MOUSE_SIDE2 => Some(2),
        _ => None,
    }
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

fn reconcile_pressed_inputs(capture_all: bool, current_key: &str) {
    if capture_all {
        for virtual_key in 0_u32..=254 {
            if matches!(virtual_key, 0x10..=0x12) {
                continue;
            }
            let Some(name) = key_name(virtual_key, 0, 0) else {
                continue;
            };
            if name == current_key {
                continue;
            }
            reconcile_key_state(
                pressed_state_code(name, virtual_key),
                async_key_is_pressed(virtual_key),
            );
        }
        reconcile_mouse_inputs(
            FILTER_MOUSE_MIDDLE | FILTER_MOUSE_SIDE1 | FILTER_MOUSE_SIDE2,
            current_key,
        );
        return;
    }

    for (word_index, filter) in CALIBRATION_KEY_FILTER.iter().enumerate() {
        let mut remaining = filter.load(Ordering::Acquire);
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            let virtual_key = (word_index * 64 + bit) as u32;
            if let Some(name) = key_name(virtual_key, 0, 0) {
                if name != current_key {
                    reconcile_key_state(
                        pressed_state_code(name, virtual_key),
                        async_key_is_pressed(virtual_key),
                    );
                }
            }
            remaining &= remaining - 1;
        }
    }
    reconcile_mouse_inputs(
        CALIBRATION_MOUSE_FILTER.load(Ordering::Acquire),
        current_key,
    );
}

fn reconcile_mouse_inputs(filter: u8, current_key: &str) {
    for (mask, name, virtual_key) in [
        (FILTER_MOUSE_MIDDLE, "MouseMiddle", 0x04),
        (FILTER_MOUSE_SIDE1, "MouseSide1", 0x05),
        (FILTER_MOUSE_SIDE2, "MouseSide2", 0x06),
    ] {
        if filter & mask != 0 && name != current_key {
            reconcile_mouse_state(mask, async_key_is_pressed(virtual_key));
        }
    }
}

fn async_key_is_pressed(virtual_key: u32) -> bool {
    // SAFETY: GetAsyncKeyState accepts any integer virtual-key value and reads
    // process-external keyboard state without retaining pointers. Callers only
    // pass the documented 0..=255 virtual-key range.
    unsafe { GetAsyncKeyState(virtual_key as i32) as u16 & 0x8000 != 0 }
}

fn reconcile_key_state(virtual_key: u32, pressed: bool) {
    let Ok(virtual_key) = usize::try_from(virtual_key) else {
        return;
    };
    let Some(word) = PRESSED_KEYS.get(virtual_key / 64) else {
        return;
    };
    let mask = 1_u64 << (virtual_key % 64);
    let previous = if pressed {
        word.fetch_or(mask, Ordering::AcqRel)
    } else {
        word.fetch_and(!mask, Ordering::AcqRel)
    };
    if (previous & mask != 0) != pressed {
        STATE_RECONCILIATIONS.fetch_add(1, Ordering::Relaxed);
        ASYNC_STATE_CORRECTIONS.fetch_add(1, Ordering::Relaxed);
    }
}

fn reconcile_mouse_state(mask: u8, pressed: bool) {
    let previous = if pressed {
        PRESSED_MOUSE.fetch_or(mask, Ordering::AcqRel)
    } else {
        PRESSED_MOUSE.fetch_and(!mask, Ordering::AcqRel)
    };
    if (previous & mask != 0) != pressed {
        STATE_RECONCILIATIONS.fetch_add(1, Ordering::Relaxed);
        ASYNC_STATE_CORRECTIONS.fetch_add(1, Ordering::Relaxed);
    }
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
        match event {
            InputEvent::Edge(edge) => {
                QUEUE_DEPTH
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| {
                        Some(depth.saturating_sub(1))
                    })
                    .ok();
                PROCESSED_EVENTS.fetch_add(1, Ordering::Relaxed);
                let event_time = unix_time_millis();
                RELEVANT_INPUT_EDGES.fetch_add(1, Ordering::Relaxed);
                LAST_RELEVANT_INPUT_AT_UNIX_MS.store(event_time, Ordering::Relaxed);
                if edge.captured_for_binding {
                    BINDING_EVENTS_FORWARDED.fetch_add(1, Ordering::Relaxed);
                    if emit_binding_edge(&app, edge).is_err() {
                        runtime_diagnostics::record_error(
                            "input",
                            "binding_event_delivery",
                            "event_delivery_failed",
                        );
                    }
                    continue;
                }
                let routed = shortcut_state()
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .route(edge.key, edge.pressed);
                match routed {
                    Some(RoutedShortcut::Macro(binding)) => {
                        SHORTCUTS_MATCHED.fetch_add(1, Ordering::Relaxed);
                        LAST_SHORTCUT_MATCH_AT_UNIX_MS.store(event_time, Ordering::Relaxed);
                        start_native_macro(&app, binding);
                    }
                    Some(RoutedShortcut::Action(action)) => {
                        SHORTCUTS_MATCHED.fetch_add(1, Ordering::Relaxed);
                        LAST_SHORTCUT_MATCH_AT_UNIX_MS.store(event_time, Ordering::Relaxed);
                        NATIVE_ACTIONS_ROUTED.fetch_add(1, Ordering::Relaxed);
                        if app
                            .emit_to("main", "native-shortcut", NativeShortcutEvent { action })
                            .is_err()
                        {
                            runtime_diagnostics::record_error(
                                "input",
                                "shortcut_event_delivery",
                                "event_delivery_failed",
                            );
                        }
                    }
                    None => {
                        UNMATCHED_SHORTCUT_EDGES.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            InputEvent::Shutdown => break,
        }
    }
}

fn emit_binding_edge(app: &AppHandle, edge: InputEdge) -> tauri::Result<()> {
    let event = match edge.source {
        InputSource::Keyboard => "global-keydown",
        InputSource::Mouse => "global-mousedown",
        InputSource::Wheel => "global-wheel",
    };
    app.emit_to("main", event, global_input_payload(edge.key, edge.pressed))
}

fn start_native_macro(app: &AppHandle, binding: NativeMacro) {
    let Ok(guard) = input::reserve() else {
        NATIVE_MACROS_SUPPRESSED.fetch_add(1, Ordering::Relaxed);
        runtime_diagnostics::record_warning("input", "macro_start", "macro_already_running");
        return;
    };
    let started = NativeMacroEvent {
        overlay_index: binding.overlay_index,
        duration: binding.prepared.duration_ms(),
        error: None,
    };
    if app
        .emit_to("main", "native-macro-started", started.clone())
        .is_err()
    {
        runtime_diagnostics::record_warning(
            "input",
            "macro_event_delivery",
            "event_delivery_failed",
        );
    }

    let worker_app = app.clone();
    let prepared = binding.prepared;
    let overlay_index = binding.overlay_index;
    let duration = prepared.duration_ms();
    NATIVE_MACROS_STARTED.fetch_add(1, Ordering::Relaxed);
    tauri::async_runtime::spawn_blocking(move || {
        let error = input::execute_prepared_macro(&prepared, guard).err();
        LAST_MACRO_COMPLETION_AT_UNIX_MS.store(unix_time_millis(), Ordering::Relaxed);
        if let Some(error) = error.as_deref() {
            if error != "Macro was cancelled" {
                NATIVE_MACROS_FAILED.fetch_add(1, Ordering::Relaxed);
                runtime_diagnostics::record_error(
                    "input",
                    "macro_execution",
                    macro_error_code(error),
                );
            }
        } else {
            NATIVE_MACROS_COMPLETED.fetch_add(1, Ordering::Relaxed);
        }
        if worker_app
            .emit_to(
                "main",
                "native-macro-finished",
                NativeMacroEvent {
                    overlay_index,
                    duration,
                    error,
                },
            )
            .is_err()
        {
            runtime_diagnostics::record_warning(
                "input",
                "macro_event_delivery",
                "event_delivery_failed",
            );
        }
    });
}

fn macro_error_code(error: &str) -> &'static str {
    if error.contains("privilege level") || error.contains("rejected synthetic") {
        "windows_input_rejected"
    } else if error.contains("cleanup also failed") {
        "input_cleanup_failed"
    } else if error.contains("shutting down") || error.contains("cancelled") {
        "macro_cancelled"
    } else {
        "macro_execution_failed"
    }
}

fn unix_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
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
        if should_ignore_keyboard_event(event.flags.0, event.dwExtraInfo) {
            OWN_SYNTHETIC_EVENTS_IGNORED.fetch_add(1, Ordering::Relaxed);
        } else {
            if event.flags.0 & LLKHF_INJECTED_FLAG != 0 {
                EXTERNAL_INJECTED_EVENTS_ACCEPTED.fetch_add(1, Ordering::Relaxed);
            }
            if let Some(key) = key_name(event.vkCode, event.scanCode, event.flags.0) {
                let state_code = pressed_state_code(key, event.vkCode);
                match wparam.0 as u32 {
                    WM_KEYUP | WM_SYSKEYUP => mark_key_released(state_code),
                    WM_KEYDOWN | WM_SYSKEYDOWN => {
                        // LowLevelKeyboardProc runs before Windows updates the
                        // asynchronous state. If our bit is still set but the
                        // pre-event state is up, the corresponding KeyUp was
                        // missed and this is a real new press, not auto-repeat.
                        reconcile_current_key_before_down(
                            state_code,
                            async_key_is_pressed(event.vkCode),
                        );
                        if mark_key_pressed_at(state_code, event.time)
                            && key_is_relevant(event.vkCode)
                        {
                            let captured_for_binding = forwards_all_inputs();
                            reconcile_pressed_inputs(captured_for_binding, key);
                            queue_event(InputEvent::Edge(InputEdge {
                                source: InputSource::Keyboard,
                                key,
                                pressed: snapshot_pressed_inputs(),
                                captured_for_binding,
                            }));
                        }
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
        if should_ignore_mouse_event(event.flags, event.dwExtraInfo) {
            OWN_SYNTHETIC_EVENTS_IGNORED.fetch_add(1, Ordering::Relaxed);
            // SAFETY: these are the unchanged arguments supplied by Windows;
            // the hook handle is ignored by `CallNextHookEx`, so `None` is valid.
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        if event.flags & LLMHF_INJECTED_FLAG != 0 {
            EXTERNAL_INJECTED_EVENTS_ACCEPTED.fetch_add(1, Ordering::Relaxed);
        }
        match wparam.0 as u32 {
            WM_MBUTTONDOWN => {
                reconcile_current_mouse_before_down(
                    FILTER_MOUSE_MIDDLE,
                    async_key_is_pressed(0x04),
                );
                let is_new = mark_mouse_pressed_at(FILTER_MOUSE_MIDDLE, event.time);
                if is_new && mouse_is_relevant(FILTER_MOUSE_MIDDLE) {
                    let captured_for_binding = forwards_all_inputs();
                    reconcile_pressed_inputs(captured_for_binding, "MouseMiddle");
                    queue_event(InputEvent::Edge(InputEdge {
                        source: InputSource::Mouse,
                        key: "MouseMiddle",
                        pressed: snapshot_pressed_inputs(),
                        captured_for_binding,
                    }));
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
                    let virtual_key = if button == "MouseSide1" { 0x05 } else { 0x06 };
                    reconcile_current_mouse_before_down(mask, async_key_is_pressed(virtual_key));
                    let is_new = mark_mouse_pressed_at(mask, event.time);
                    if is_new && mouse_is_relevant(mask) {
                        let captured_for_binding = forwards_all_inputs();
                        reconcile_pressed_inputs(captured_for_binding, button);
                        queue_event(InputEvent::Edge(InputEdge {
                            source: InputSource::Mouse,
                            key: button,
                            pressed: snapshot_pressed_inputs(),
                            captured_for_binding,
                        }));
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
                    let key = if rotation > 0 { "WheelUp" } else { "WheelDown" };
                    let captured_for_binding = forwards_all_inputs();
                    reconcile_pressed_inputs(captured_for_binding, key);
                    queue_event(InputEvent::Edge(InputEdge {
                        source: InputSource::Wheel,
                        key,
                        pressed: snapshot_pressed_inputs(),
                        captured_for_binding,
                    }));
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
        let is_edge = matches!(event, InputEvent::Edge(_));
        let reserved_depth = is_edge.then(|| QUEUE_DEPTH.fetch_add(1, Ordering::Relaxed) + 1);
        match sender.try_send(event) {
            Ok(()) => {
                if let Some(depth) = reserved_depth {
                    QUEUED_EVENTS.fetch_add(1, Ordering::Relaxed);
                    MAX_QUEUE_DEPTH
                        .fetch_max(depth.min(EVENT_QUEUE_CAPACITY as u64), Ordering::Relaxed);
                }
            }
            Err(_) => {
                if reserved_depth.is_some() {
                    QUEUE_DEPTH
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| {
                            Some(depth.saturating_sub(1))
                        })
                        .ok();
                    DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

fn should_ignore_keyboard_event(flags: u32, extra_info: usize) -> bool {
    flags & LLKHF_INJECTED_FLAG != 0 && extra_info == input::SYNTHETIC_INPUT_MARKER
}

fn should_ignore_mouse_event(flags: u32, extra_info: usize) -> bool {
    flags & LLMHF_INJECTED_FLAG != 0 && extra_info == input::SYNTHETIC_INPUT_MARKER
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

    fn macro_payload() -> input::MacroPayload {
        input::MacroPayload {
            menu_key: Some("ControlLeft".to_owned()),
            menu_mode: Some("hold".to_owned()),
            direction_only: Some(true),
            sequence: vec!["KeyW".to_owned(), "KeyA".to_owned()],
            menu_open_delay: Some(100),
            press_delay: Some(10),
            interval_delay: Some(10),
        }
    }

    fn macro_binding(hotkey: &str, overlay_index: i64) -> MacroShortcutConfig {
        MacroShortcutConfig {
            hotkey: hotkey.to_owned(),
            payload: macro_payload(),
            overlay_index,
        }
    }

    fn shortcut_config(
        macros: Vec<MacroShortcutConfig>,
        ocr_hotkey: Option<&str>,
        overlay_visible: bool,
        overlay_exec: Option<&str>,
    ) -> ShortcutConfig {
        ShortcutConfig {
            macros,
            ocr_hotkey: ocr_hotkey.map(str::to_owned),
            overlay_visible,
            overlay_up: None,
            overlay_down: None,
            overlay_exec: overlay_exec.map(str::to_owned),
        }
    }

    fn pressed_inputs(names: &[&str]) -> PressedInputs {
        let mut pressed = PressedInputs::default();
        let mut ignored_calibration = InputFilter::default();
        for name in names {
            add_required_input(&mut pressed, &mut ignored_calibration, name)
                .expect("supported pressed input");
        }
        pressed
    }

    fn routed_macro_index(route: Option<RoutedShortcut>) -> Option<i64> {
        match route {
            Some(RoutedShortcut::Macro(binding)) => Some(binding.overlay_index),
            Some(RoutedShortcut::Action(_)) | None => None,
        }
    }

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
    fn native_matching_allows_extra_held_inputs_and_requires_the_final_trigger() {
        let state = ShortcutState::build(shortcut_config(
            vec![macro_binding("ControlLeft+Digit1", 7)],
            None,
            false,
            None,
        ))
        .expect("valid native shortcut table");
        let held = pressed_inputs(&["ShiftLeft", "KeyW", "ControlLeft", "Digit1"]);

        assert_eq!(routed_macro_index(state.route("Digit1", held)), Some(7));
        assert_eq!(routed_macro_index(state.route("ControlLeft", held)), None);
        assert_eq!(
            routed_macro_index(state.route("Digit1", pressed_inputs(&["Digit1"]))),
            None
        );
    }

    #[test]
    fn native_matching_prefers_the_most_specific_overlapping_chord() {
        let state = ShortcutState::build(shortcut_config(
            vec![
                macro_binding("ControlLeft+Digit1", 1),
                macro_binding("ControlLeft+ShiftLeft+Digit1", 2),
            ],
            None,
            false,
            None,
        ))
        .expect("valid native shortcut table");
        let held = pressed_inputs(&["ControlLeft", "ShiftLeft", "KeyW", "Digit1"]);

        assert_eq!(routed_macro_index(state.route("Digit1", held)), Some(2));
    }

    #[test]
    fn native_matching_preserves_chord_and_ui_action_precedence() {
        let state = ShortcutState::build(shortcut_config(
            vec![macro_binding("ControlLeft+F8", 1), macro_binding("F8", 2)],
            Some("F8"),
            true,
            Some("F8"),
        ))
        .expect("valid native shortcut table");

        assert_eq!(
            routed_macro_index(state.route("F8", pressed_inputs(&["ControlLeft", "F8"]))),
            Some(1),
            "a multi-key macro gets first chance at a shared trigger"
        );
        assert!(matches!(
            state.route("F8", pressed_inputs(&["F8"])),
            Some(RoutedShortcut::Action("ocr"))
        ));

        let overlay_state = ShortcutState::build(shortcut_config(
            vec![macro_binding("F8", 3)],
            None,
            true,
            Some("F8"),
        ))
        .expect("valid overlay shortcut table");
        assert!(matches!(
            overlay_state.route("F8", pressed_inputs(&["F8"])),
            Some(RoutedShortcut::Action("overlay-exec"))
        ));
    }

    #[test]
    fn validates_chords_and_precompiles_macro_payloads() {
        assert!(parse_shortcut_chord("WheelUp+Digit1").is_err());
        assert!(parse_shortcut_chord("ControlLeft+WheelUp").is_ok());
        assert!(parse_shortcut_chord("ControlLeft+ControlLeft").is_err());
        assert!(parse_shortcut_chord("NotARealKey").is_err());
        assert!(parse_shortcut_chord(
            "ControlLeft+ShiftLeft+AltLeft+MetaLeft+KeyW+KeyA+KeyS+KeyD+Digit1"
        )
        .is_err());

        let mut invalid = macro_binding("F8", 0);
        invalid.payload.sequence = vec!["NotARealKey".to_owned()];
        assert!(ShortcutState::build(shortcut_config(vec![invalid], None, false, None)).is_err());
    }

    #[test]
    fn deserializes_the_frontend_native_shortcut_contract() {
        let config: ShortcutConfig = serde_json::from_value(serde_json::json!({
            "macros": [{
                "hotkey": "ControlLeft+Digit1",
                "payload": {
                    "menuKey": "ControlLeft",
                    "menuMode": "hold",
                    "directionOnly": false,
                    "sequence": ["KeyW", "KeyA"],
                    "menuOpenDelay": 100,
                    "pressDelay": 10,
                    "intervalDelay": 10
                },
                "overlayIndex": 4
            }],
            "ocrHotkey": "F8",
            "overlayVisible": true,
            "overlayUp": "ArrowUp",
            "overlayDown": "ArrowDown",
            "overlayExec": "Enter"
        }))
        .expect("frontend shortcut payload");
        let state = ShortcutState::build(config).expect("precompiled frontend shortcut state");

        assert_eq!(state.macros.len(), 1);
        assert_eq!(state.macros[0].overlay_index, 4);
        assert_eq!(state.ocr_hotkey, Some("F8"));
        assert_eq!(state.overlay_exec, Some("Enter"));
    }

    #[test]
    fn enter_variants_are_not_guessed_by_async_state_calibration() {
        let (main_enter, main_calibration) =
            parse_shortcut_chord("Enter+Digit1").expect("main Enter chord");
        let (numpad_enter, numpad_calibration) =
            parse_shortcut_chord("NumpadEnter+Digit1").expect("numpad Enter chord");

        assert!(main_enter.required.contains(pressed_inputs(&["Enter"])));
        assert!(numpad_enter
            .required
            .contains(pressed_inputs(&["NumpadEnter"])));
        assert!(!main_enter
            .required
            .contains(pressed_inputs(&["NumpadEnter"])));
        assert!(!numpad_enter.required.contains(pressed_inputs(&["Enter"])));
        assert_eq!(main_calibration, InputFilter::default());
        assert_eq!(numpad_calibration, InputFilter::default());
    }

    #[test]
    fn stale_pressed_edges_recover_after_missed_keyup() {
        assert!(repeated_edge_is_recovered(0, 100));
        assert!(!repeated_edge_is_recovered(100, 4_999));
        assert!(repeated_edge_is_recovered(100, 5_100));
        assert!(repeated_edge_is_recovered(u32::MAX - 100, 5_000));
    }

    #[test]
    fn current_down_calibration_recovers_a_missed_release_without_waiting() {
        let _state_guard = FILTER_UPDATE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reset_pressed_inputs();

        assert!(mark_key_pressed(0x57));
        reconcile_current_key_before_down(0x57, true);
        assert!(!mark_key_pressed(0x57), "auto-repeat must stay suppressed");
        reconcile_current_key_before_down(0x57, false);
        assert!(
            mark_key_pressed(0x57),
            "a missed KeyUp must recover immediately"
        );

        assert!(mark_mouse_pressed(FILTER_MOUSE_SIDE1));
        reconcile_current_mouse_before_down(FILTER_MOUSE_SIDE1, true);
        assert!(!mark_mouse_pressed(FILTER_MOUSE_SIDE1));
        reconcile_current_mouse_before_down(FILTER_MOUSE_SIDE1, false);
        assert!(mark_mouse_pressed(FILTER_MOUSE_SIDE1));
        reset_pressed_inputs();
    }

    #[test]
    fn rejects_unknown_filter_keys_and_suppresses_key_repeat() {
        assert!(build_filter(&["NotARealKey".to_owned()]).is_err());

        let _state_guard = FILTER_UPDATE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reset_pressed_inputs();
        assert!(mark_key_pressed_at(0x57, 100));
        assert!(!mark_key_pressed_at(0x57, 200));
        assert!(mark_key_pressed_at(0x57, 5_200));
        mark_key_released(0x57);
        assert!(mark_key_pressed_at(0x57, 5_300));
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
    fn ignores_only_input_generated_by_this_process() {
        assert!(should_ignore_keyboard_event(
            LLKHF_INJECTED_FLAG,
            input::SYNTHETIC_INPUT_MARKER
        ));
        assert!(should_ignore_mouse_event(
            LLMHF_INJECTED_FLAG,
            input::SYNTHETIC_INPUT_MARKER
        ));

        // Third-party injected input can be legitimate input from keyboard
        // drivers, remappers, remote desktops, and accessibility software.
        assert!(!should_ignore_keyboard_event(LLKHF_INJECTED_FLAG, 0));
        assert!(!should_ignore_mouse_event(LLMHF_INJECTED_FLAG, 0));
        assert!(!should_ignore_keyboard_event(
            0,
            input::SYNTHETIC_INPUT_MARKER
        ));
        assert!(!should_ignore_mouse_event(0, input::SYNTHETIC_INPUT_MARKER));
    }

    #[test]
    fn classifies_macro_failures_without_exporting_raw_error_text() {
        assert_eq!(
            macro_error_code("Windows rejected synthetic keyboard input; check privilege level"),
            "windows_input_rejected"
        );
        assert_eq!(
            macro_error_code("primary failure; input cleanup also failed: secondary"),
            "input_cleanup_failed"
        );
        assert_eq!(macro_error_code("Macro was cancelled"), "macro_cancelled");
        assert_eq!(
            macro_error_code("some future internal failure"),
            "macro_execution_failed"
        );
    }
}
