#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod catalog;
mod config;
mod hooks;
mod input;
mod legacy;
mod ocr;
mod tray;
mod updates;
mod windows;

use config::MigrationReport;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

struct AppState {
    data_dir: PathBuf,
    data_io: Mutex<()>,
    ocr_model_dir: PathBuf,
    migration_report: MigrationReport,
    ocr_engine: Mutex<Option<ocr::OcrEngine>>,
    ocr_running: AtomicBool,
    exit_started: AtomicBool,
    exit_fallback_scheduled: AtomicBool,
    ocr_selection_display: Mutex<Option<ocr::OcrDisplay>>,
    window_creation: Mutex<()>,
    window_payloads: Mutex<WindowPayloads>,
    overlay_snapshot: Mutex<OverlaySnapshot>,
    last_toast: Mutex<Option<Value>>,
    toast_generation: AtomicU64,
}

struct WindowPayloads {
    ocr_help_language: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlaySnapshot {
    settings: Value,
    data: Value,
    selection: i64,
    locked: bool,
}

const DEFAULT_SPONSOR_URL: &str =
    "https://www.yifut.com/paypage/?merchant=a0ccz04gJj%2BJNsdjP9cTbIj2MrN958lGiZ7Ub2SdvLGZ";

fn resolve_ocr_model_dir(app: &AppHandle) -> PathBuf {
    use std::env;

    #[cfg(debug_assertions)]
    let development_models = option_env!("CARGO_MANIFEST_DIR")
        .map(|dir| PathBuf::from(dir).join("resources/models/ocr"));
    #[cfg(not(debug_assertions))]
    let development_models: Option<PathBuf> = None;

    let candidates = [
        // 1. Tauri resource_dir (works when properly installed/bundled)
        app.path()
            .resource_dir()
            .ok()
            .map(|dir| dir.join("models/ocr")),
        // 2. <exe_dir>/resources/models/ocr (installed alongside exe)
        env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("resources/models/ocr"))),
        // 3. <exe_dir>/models/ocr (direct run fallback)
        env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("models/ocr"))),
        // 4. CARGO_MANIFEST_DIR/resources/models/ocr (dev mode)
        development_models,
    ];

    for candidate in candidates.iter().flatten() {
        if ocr::model_files_exist(candidate) {
            return candidate.clone();
        }
    }

    // Fallback: return the resource_dir path even if it doesn't exist yet
    // so the error message is meaningful
    app.path()
        .resource_dir()
        .map(|dir| dir.join("models/ocr"))
        .unwrap_or_else(|_| {
            env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|p| p.join("resources/models/ocr")))
                .unwrap_or_default()
        })
}

struct OcrRunGuard {
    app: AppHandle,
}

impl OcrRunGuard {
    fn acquire(app: AppHandle) -> Result<Self, String> {
        app.state::<AppState>()
            .ocr_running
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| "OCR is already running".to_owned())?;
        Ok(Self { app })
    }
}

impl Drop for OcrRunGuard {
    fn drop(&mut self) {
        self.app
            .state::<AppState>()
            .ocr_running
            .store(false, Ordering::Release);
    }
}

fn lock_ocr_engine(state: &AppState) -> MutexGuard<'_, Option<ocr::OcrEngine>> {
    match state.ocr_engine.lock() {
        Ok(engine) => engine,
        Err(poisoned) => {
            let mut engine = poisoned.into_inner();
            *engine = None;
            state.ocr_engine.clear_poison();
            engine
        }
    }
}

#[tauri::command]
fn load_data(state: State<'_, AppState>, filename: String) -> Result<Option<Value>, String> {
    config::load_json(&state.data_dir, &filename)
}

#[tauri::command]
fn save_data(state: State<'_, AppState>, filename: String, data: Value) -> Result<(), String> {
    let _data_io = state
        .data_io
        .lock()
        .map_err(|_| "Data storage is unavailable".to_owned())?;
    if filename != "settings.json" {
        return config::save_json(&state.data_dir, &filename, &data);
    }
    let data = preserve_overlay_position(&state.data_dir, data)?;
    config::save_json(&state.data_dir, &filename, &data)
}

fn load_overlay_position(
    data_dir: &std::path::Path,
) -> Result<Option<windows::OverlayPosition>, String> {
    let Some(settings) = config::load_json(data_dir, "settings.json")? else {
        return Ok(None);
    };
    Ok(load_overlay_position_from_value(
        settings.get("overlayPosition"),
    ))
}

fn preserve_overlay_position(data_dir: &std::path::Path, mut data: Value) -> Result<Value, String> {
    let Some(settings) = data.as_object_mut() else {
        return Err("Settings must be a JSON object".to_owned());
    };
    let Some(saved_position) = load_overlay_position(data_dir)? else {
        return Ok(data);
    };
    if load_overlay_position_from_value(settings.get("overlayPosition")).is_none() {
        settings.insert(
            "overlayPosition".to_owned(),
            json!({ "x": saved_position.x, "y": saved_position.y }),
        );
    }
    Ok(data)
}

fn load_overlay_position_from_value(value: Option<&Value>) -> Option<windows::OverlayPosition> {
    let position = value?;
    let x = i32::try_from(position.get("x")?.as_i64()?).ok()?;
    let y = i32::try_from(position.get("y")?.as_i64()?).ok()?;
    const MAX_ABSOLUTE_POSITION: i32 = 100_000;
    ((-MAX_ABSOLUTE_POSITION..=MAX_ABSOLUTE_POSITION).contains(&x)
        && (-MAX_ABSOLUTE_POSITION..=MAX_ABSOLUTE_POSITION).contains(&y))
    .then_some(windows::OverlayPosition { x, y })
}

fn persist_overlay_position(
    app: &AppHandle,
    position: windows::OverlayPosition,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _data_io = state
        .data_io
        .lock()
        .map_err(|_| "Data storage is unavailable".to_owned())?;
    let data_dir = state.data_dir.clone();
    let mut settings = config::load_json(&data_dir, "settings.json")?
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let Some(values) = settings.as_object_mut() else {
        return Err("Saved settings are not a JSON object".to_owned());
    };
    values.insert(
        "overlayPosition".to_owned(),
        json!({ "x": position.x, "y": position.y }),
    );
    config::save_json(&data_dir, "settings.json", &settings)
}

pub(crate) fn persist_current_overlay_position(app: &AppHandle) -> Result<(), String> {
    if let Some(position) = windows::get_overlay_position(app)? {
        persist_overlay_position(app, position)?;
    }
    Ok(())
}

#[tauri::command]
fn migration_status(state: State<'_, AppState>) -> Result<MigrationReport, String> {
    Ok(state.migration_report.clone())
}

#[tauri::command]
fn set_global_input_filter(config: hooks::ShortcutConfig, capture_all: bool) -> Result<(), String> {
    hooks::configure(config, capture_all)
}

#[tauri::command]
fn get_input_diagnostics() -> hooks::InputDiagnostics {
    hooks::diagnostics()
}

#[tauri::command]
async fn toggle_overlay(app: AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    // WebviewWindowBuilder::build waits for the Windows event loop. Tauri
    // synchronous commands run on that same loop, so creating an on-demand
    // window from a sync command deadlocks the whole application.
    let _creation = state
        .window_creation
        .lock()
        .map_err(|_| "Window creation is unavailable".to_owned())?;
    let saved_position = load_overlay_position(&state.data_dir)?;
    let visible = windows::toggle_overlay(&app, saved_position)?;
    if !visible {
        persist_current_overlay_position(&app)?;
    }
    Ok(visible)
}

#[tauri::command]
fn window_minimize(app: AppHandle) -> Result<(), String> {
    windows::minimize_main(&app)
}

#[tauri::command]
fn window_tray(app: AppHandle) -> Result<(), String> {
    windows::hide_main(&app)
}

#[tauri::command]
fn window_close(app: AppHandle) -> Result<(), String> {
    exit_app(&app)
}

#[tauri::command]
fn begin_exit(app: AppHandle) {
    if !app.state::<AppState>().exit_started.load(Ordering::Acquire) {
        schedule_force_exit(&app);
    }
}

fn exit_app(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.exit_started.load(Ordering::Acquire) {
        return Ok(());
    }
    schedule_force_exit(app);
    // Position persistence is best-effort at shutdown, but cancellation and
    // hook cleanup must run even when the settings file is unavailable.
    let persist_result = persist_current_overlay_position(app);
    force_exit(app);
    persist_result
}

fn force_exit(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.exit_started.swap(true, Ordering::AcqRel) {
        return;
    }
    input::cancel_and_wait(std::time::Duration::from_millis(250));
    hooks::stop();
    app.exit(0);
}

fn schedule_force_exit(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !claim_exit_fallback_slot(&state.exit_fallback_scheduled) {
        return;
    }

    // File-system calls and a crashed renderer must never leave the process
    // half-closed. Every graceful exit path shares this single watchdog.
    let fallback_app = app.clone();
    if std::thread::Builder::new()
        .name("quit-fallback".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(2));
            force_exit(&fallback_app);
        })
        .is_err()
    {
        force_exit(app);
    }
}

fn claim_exit_fallback_slot(flag: &AtomicBool) -> bool {
    !flag.swap(true, Ordering::AcqRel)
}

pub(crate) fn request_exit_from_tray(app: &AppHandle) -> Result<(), String> {
    if app.emit_to("main", "quit-requested", ()).is_err() {
        force_exit(app);
        return Ok(());
    }

    // Give the renderer time to flush its coalesced save queue. If it has not
    // loaded yet or has crashed, this bounded fallback still guarantees exit.
    schedule_force_exit(app);
    Ok(())
}

#[tauri::command]
fn lock_overlay(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    windows::lock_overlay(&app, true)?;
    state
        .overlay_snapshot
        .lock()
        .map_err(|_| "Overlay state is unavailable".to_owned())?
        .locked = true;
    Ok(())
}

#[tauri::command]
fn unlock_overlay(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    windows::lock_overlay(&app, false)?;
    state
        .overlay_snapshot
        .lock()
        .map_err(|_| "Overlay state is unavailable".to_owned())?
        .locked = false;
    Ok(())
}

#[tauri::command]
fn resize_overlay(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    windows::resize_overlay(&app, width, height)
}

#[tauri::command]
fn set_overlay_position(app: AppHandle, position: windows::OverlayPosition) -> Result<(), String> {
    windows::set_overlay_position(&app, position)?;
    persist_overlay_position(&app, position)
}

#[tauri::command]
fn update_overlay_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Value,
) -> Result<(), String> {
    state
        .overlay_snapshot
        .lock()
        .map_err(|_| "Overlay state is unavailable".to_owned())?
        .settings = settings.clone();
    windows::emit_overlay(&app, "overlay-settings", settings)
}

#[tauri::command]
fn update_overlay(app: AppHandle, state: State<'_, AppState>, data: Value) -> Result<(), String> {
    state
        .overlay_snapshot
        .lock()
        .map_err(|_| "Overlay state is unavailable".to_owned())?
        .data = data.clone();
    windows::emit_overlay(&app, "render-overlay", data)
}

#[tauri::command]
fn highlight_overlay(app: AppHandle, data: Value) -> Result<(), String> {
    windows::emit_overlay(&app, "highlight-item", data)
}

#[tauri::command]
fn update_selection(app: AppHandle, state: State<'_, AppState>, index: i64) -> Result<(), String> {
    state
        .overlay_snapshot
        .lock()
        .map_err(|_| "Overlay state is unavailable".to_owned())?
        .selection = index;
    windows::emit_overlay(&app, "selection-changed", Value::from(index))
}

#[tauri::command]
fn get_overlay_snapshot(state: State<'_, AppState>) -> Result<OverlaySnapshot, String> {
    state
        .overlay_snapshot
        .lock()
        .map(|snapshot| snapshot.clone())
        .map_err(|_| "Overlay state is unavailable".to_owned())
}

#[tauri::command]
async fn show_toast(
    app: AppHandle,
    state: State<'_, AppState>,
    mut payload: Value,
) -> Result<(), String> {
    let _creation = state
        .window_creation
        .lock()
        .map_err(|_| "Window creation is unavailable".to_owned())?;
    let generation = state.toast_generation.fetch_add(1, Ordering::AcqRel) + 1;
    if let Some(object) = payload.as_object_mut() {
        object.insert("generation".to_owned(), Value::from(generation));
    }
    *state
        .last_toast
        .lock()
        .map_err(|_| "Toast state is unavailable".to_owned())? = Some(payload.clone());
    if let Err(error) = windows::show_toast(&app, payload) {
        let _ = windows::hide_toast(&app);
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
fn hide_toast(app: AppHandle, state: State<'_, AppState>, generation: u64) -> Result<(), String> {
    if state.toast_generation.load(Ordering::Acquire) != generation {
        return Ok(());
    }
    windows::hide_toast(&app)
}

#[tauri::command]
fn get_last_toast(state: State<'_, AppState>) -> Result<Option<Value>, String> {
    state
        .last_toast
        .lock()
        .map(|payload| payload.clone())
        .map_err(|_| "Toast state is unavailable".to_owned())
}

#[tauri::command]
async fn execute_macro(payload: input::MacroPayload) -> Result<(), String> {
    if payload.sequence.is_empty() {
        return Ok(());
    }
    let guard = input::reserve()?;
    tauri::async_runtime::spawn_blocking(move || input::execute_reserved(payload, guard))
        .await
        .map_err(|error| format!("Macro task failed: {error}"))?
}

#[tauri::command]
async fn open_sponsor(app: AppHandle) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let _creation = state
        .window_creation
        .lock()
        .map_err(|_| "Window creation is unavailable".to_owned())?;
    windows::open_sponsor(&app, DEFAULT_SPONSOR_URL.to_owned())?;
    Ok(true)
}

#[tauri::command]
async fn close_sponsor_window(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let _creation = state
        .window_creation
        .lock()
        .map_err(|_| "Window creation is unavailable".to_owned())?;
    windows::destroy_window(&app, "sponsor")
}

#[tauri::command]
fn get_sponsor_url() -> Result<String, String> {
    Ok(DEFAULT_SPONSOR_URL.to_owned())
}

#[tauri::command]
async fn open_ocr_help(
    app: AppHandle,
    state: State<'_, AppState>,
    language: String,
) -> Result<bool, String> {
    let _creation = state
        .window_creation
        .lock()
        .map_err(|_| "Window creation is unavailable".to_owned())?;
    let language = if language == "en" {
        "en".to_owned()
    } else {
        "zh".to_owned()
    };
    state
        .window_payloads
        .lock()
        .map_err(|_| "OCR help state is unavailable".to_owned())?
        .ocr_help_language = language.clone();
    windows::open_ocr_help(&app, language)?;
    Ok(true)
}

#[tauri::command]
async fn close_ocr_help_window(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let _creation = state
        .window_creation
        .lock()
        .map_err(|_| "Window creation is unavailable".to_owned())?;
    windows::destroy_window(&app, "ocr-help")
}

#[tauri::command]
fn get_ocr_help_language(state: State<'_, AppState>) -> Result<String, String> {
    state
        .window_payloads
        .lock()
        .map(|payloads| payloads.ocr_help_language.clone())
        .map_err(|_| "OCR help state is unavailable".to_owned())
}

#[tauri::command]
fn get_app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
async fn check_for_updates() -> Option<updates::UpdateInfo> {
    match tauri::async_runtime::spawn_blocking(updates::check_for_update).await {
        Ok(Ok(update)) => update,
        Ok(Err(_error)) => {
            #[cfg(debug_assertions)]
            eprintln!("Update check skipped: {_error}");
            None
        }
        Err(_error) => {
            #[cfg(debug_assertions)]
            eprintln!("Update check task failed: {_error}");
            None
        }
    }
}

#[tauri::command]
fn open_release_download() -> Result<(), String> {
    updates::open_releases_page()
}

#[tauri::command]
async fn fetch_remote_builtin_strats() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(catalog::fetch_remote_builtin_strats)
        .await
        .map_err(|error| format!("Catalog download task failed: {error}"))?
}

#[tauri::command]
async fn ocr_model_status(app: AppHandle) -> Result<ocr::ModelStatus, String> {
    let guard = OcrRunGuard::acquire(app.clone())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let state = app.state::<AppState>();
        let mut engine_slot = lock_ocr_engine(&state);
        if engine_slot.is_none() {
            *engine_slot = Some(ocr::OcrEngine::load_from_directory(
                state.ocr_model_dir.clone(),
            )?);
        }
        engine_slot
            .as_ref()
            .map(ocr::OcrEngine::model_status)
            .ok_or_else(|| "OCR engine failed to initialize".to_owned())
    })
    .await
    .map_err(|error| format!("OCR status task failed: {error}"))?
}

#[tauri::command]
async fn recognize_ocr_region(
    app: AppHandle,
    region: ocr::OcrRegion,
) -> Result<ocr::OcrText, String> {
    let guard = OcrRunGuard::acquire(app.clone())?;
    let state = app.state::<AppState>();
    let model_dir = state.ocr_model_dir.clone();

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<ocr::OcrText, String> {
        let _guard = guard;
        let state = app.state::<AppState>();
        // Serialize before capture so concurrent IPC cannot retain multiple
        // full-size desktop frames while waiting for the inference engine.
        let mut engine_slot = lock_ocr_engine(&state);
        let frame = ocr::capture_region(region)?;
        if engine_slot.is_none() {
            *engine_slot = Some(ocr::OcrEngine::load_from_directory(model_dir)?);
        }
        engine_slot
            .as_mut()
            .ok_or_else(|| "OCR engine failed to initialize".to_owned())?
            .recognize_region(frame)
    })
    .await
    .map_err(|error| format!("OCR task failed: {error}"))?;
    result
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OcrSelectionEvent {
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
}

#[tauri::command]
fn get_ocr_displays() -> Result<Vec<ocr::OcrDisplay>, String> {
    ocr::available_displays()
}

#[tauri::command]
async fn start_ocr_region_select(
    app: AppHandle,
    state: State<'_, AppState>,
    display_id: Option<u32>,
) -> Result<(), String> {
    let _creation = state
        .window_creation
        .lock()
        .map_err(|_| "Window creation is unavailable".to_owned())?;
    let displays = ocr::available_displays()?;
    let display = display_id
        .and_then(|id| displays.iter().find(|display| display.id == id))
        .or_else(|| displays.iter().find(|display| display.is_primary))
        .or_else(|| displays.first())
        .cloned()
        .ok_or_else(|| "No OCR display is available".to_owned())?;
    *state
        .ocr_selection_display
        .lock()
        .map_err(|_| "OCR selection state is unavailable".to_owned())? = Some(display.clone());
    if let Err(error) = windows::open_ocr_select(&app, &display) {
        *state
            .ocr_selection_display
            .lock()
            .map_err(|_| "OCR selection state is unavailable".to_owned())? = None;
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn ocr_region_selected(
    app: AppHandle,
    state: State<'_, AppState>,
    region: OcrSelectionEvent,
) -> Result<(), String> {
    let _creation = state
        .window_creation
        .lock()
        .map_err(|_| "Window creation is unavailable".to_owned())?;
    let display = state
        .ocr_selection_display
        .lock()
        .map_err(|_| "OCR selection state is unavailable".to_owned())?
        .take()
        .ok_or_else(|| "OCR selection display is unavailable".to_owned())?;
    let normalized = normalize_ocr_selection(&display, region);
    let emit_result = app
        .emit_to("main", "ocr-region-selected", normalized)
        .map_err(|error| error.to_string());
    let destroy_result = windows::destroy_window(&app, "ocr-select");
    emit_result.and(destroy_result)
}

fn normalize_ocr_selection(display: &ocr::OcrDisplay, region: OcrSelectionEvent) -> ocr::OcrRegion {
    // Browser pointer coordinates are logical CSS pixels, while DisplayInfo and
    // screenshots use physical desktop pixels. Convert exactly once here.
    let scale = display.scale_factor as f64;
    ocr::OcrRegion {
        x: (display.bounds.x as f64 + region.start_x.min(region.end_x) * scale).round(),
        y: (display.bounds.y as f64 + region.start_y.min(region.end_y) * scale).round(),
        width: ((region.end_x - region.start_x).abs() * scale).round(),
        height: ((region.end_y - region.start_y).abs() * scale).round(),
    }
}

#[tauri::command]
async fn cancel_ocr_region_select(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _creation = state
        .window_creation
        .lock()
        .map_err(|_| "Window creation is unavailable".to_owned())?;
    *state
        .ocr_selection_display
        .lock()
        .map_err(|_| "OCR selection state is unavailable".to_owned())? = None;
    windows::destroy_window(&app, "ocr-select")
}

fn main() {
    if !legacy::preflight_allows_startup() {
        return;
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    // Electron's main-window close path hid to tray and only
                    // quit on the tray "quit" menu. Mirror that so auxiliary
                    // overlay/toast windows stay alive while the app runs.
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let ocr_model_dir = resolve_ocr_model_dir(app.handle());
            let migration_report =
                config::initialize_data_directory(&data_dir).map_err(std::io::Error::other)?;
            app.manage(AppState {
                data_dir,
                data_io: Mutex::new(()),
                ocr_model_dir,
                migration_report,
                ocr_engine: Mutex::new(None),
                ocr_running: AtomicBool::new(false),
                exit_started: AtomicBool::new(false),
                exit_fallback_scheduled: AtomicBool::new(false),
                ocr_selection_display: Mutex::new(None),
                window_creation: Mutex::new(()),
                window_payloads: Mutex::new(WindowPayloads {
                    ocr_help_language: "zh".to_owned(),
                }),
                overlay_snapshot: Mutex::new(OverlaySnapshot {
                    settings: Value::Object(serde_json::Map::new()),
                    data: Value::Array(Vec::new()),
                    selection: 0,
                    locked: false,
                }),
                last_toast: Mutex::new(None),
                toast_generation: AtomicU64::new(0),
            });
            tray::create(app.handle())?;
            hooks::start(app.handle().clone()).map_err(std::io::Error::other)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_data,
            save_data,
            migration_status,
            set_global_input_filter,
            get_input_diagnostics,
            toggle_overlay,
            window_minimize,
            window_tray,
            begin_exit,
            window_close,
            lock_overlay,
            unlock_overlay,
            resize_overlay,
            set_overlay_position,
            update_overlay_settings,
            update_overlay,
            highlight_overlay,
            update_selection,
            get_overlay_snapshot,
            show_toast,
            hide_toast,
            get_last_toast,
            execute_macro,
            open_sponsor,
            close_sponsor_window,
            get_sponsor_url,
            open_ocr_help,
            close_ocr_help_window,
            get_ocr_help_language,
            get_app_version,
            check_for_updates,
            open_release_download,
            fetch_remote_builtin_strats,
            ocr_model_status,
            recognize_ocr_region,
            get_ocr_displays,
            start_ocr_region_select,
            ocr_region_selected,
            cancel_ocr_region_select
        ])
        .run(tauri::generate_context!())
        .expect("error while running HD2 Macro Terminal Rust");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn preserves_a_dragged_overlay_position_when_other_settings_are_saved() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!(
            "hd2-macro-terminal-rust-overlay-position-{unique}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&data_dir).expect("temporary app data directory should be creatable");
        config::save_json(
            &data_dir,
            "settings.json",
            &json!({ "overlayPosition": { "x": 320, "y": 240 }, "ovOpacity": 90 }),
        )
        .expect("saved overlay position should be writable");

        let merged = preserve_overlay_position(&data_dir, json!({ "ovOpacity": 55 }))
            .expect("settings merge should succeed");
        assert_eq!(
            load_overlay_position_from_value(merged.get("overlayPosition")),
            Some(windows::OverlayPosition { x: 320, y: 240 })
        );
        assert_eq!(merged.get("ovOpacity"), Some(&Value::from(55)));

        fs::remove_dir_all(data_dir).expect("temporary app data directory should be removable");
    }

    #[test]
    fn ocr_selection_converts_css_coordinates_to_physical_pixels_at_high_dpi() {
        let display = ocr::OcrDisplay {
            id: 1,
            index: 1,
            bounds: ocr::OcrBounds {
                x: -1280,
                y: 240,
                width: 2560,
                height: 1440,
            },
            scale_factor: 1.5,
            is_primary: false,
        };
        let selected = normalize_ocr_selection(
            &display,
            OcrSelectionEvent {
                start_x: 400.25,
                start_y: 300.5,
                end_x: 40.75,
                end_y: 600.25,
            },
        );

        assert_eq!(selected.x, -1219.0);
        assert_eq!(selected.y, 691.0);
        assert_eq!(selected.width, 539.0);
        assert_eq!(selected.height, 450.0);
    }

    #[test]
    fn only_one_exit_watchdog_can_be_claimed_concurrently() {
        let claimed = std::sync::Arc::new(AtomicBool::new(false));
        let winners = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let workers = (0..16)
            .map(|_| {
                let claimed = std::sync::Arc::clone(&claimed);
                let winners = std::sync::Arc::clone(&winners);
                std::thread::spawn(move || {
                    if claim_exit_fallback_slot(&claimed) {
                        winners.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().expect("watchdog claimant should not panic");
        }
        assert_eq!(winners.load(Ordering::Relaxed), 1);
    }
}
