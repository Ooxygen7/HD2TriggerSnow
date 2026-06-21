#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod hooks;
mod input;
mod ocr;
mod tray;
mod windows;

use config::MigrationReport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, sync::Mutex};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

struct AppState {
    data_dir: PathBuf,
    ocr_model_dir: PathBuf,
    migration_report: Mutex<MigrationReport>,
    ocr_engine: Mutex<Option<ocr::OcrEngine>>,
    ocr_selection_display: Mutex<Option<ocr::OcrDisplay>>,
    window_payloads: Mutex<WindowPayloads>,
    overlay_snapshot: Mutex<OverlaySnapshot>,
    last_toast: Mutex<Option<Value>>,
    is_quitting: Mutex<bool>,
}

struct WindowPayloads {
    sponsor_url: String,
    ocr_help_language: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlaySnapshot {
    settings: Value,
    data: Value,
    selection: i64,
}

const DEFAULT_SPONSOR_URL: &str =
    "https://www.yifut.com/paypage/?merchant=a0ccz04gJj%2BJNsdjP9cTbIj2MrN958lGiZ7Ub2SdvLGZ";

fn resolve_ocr_model_dir(app: &AppHandle) -> PathBuf {
    use std::env;

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
        option_env!("CARGO_MANIFEST_DIR")
            .map(|dir| PathBuf::from(dir).join("resources/models/ocr")),
    ];

    for candidate in candidates.iter().flatten() {
        if candidate.join("PP-OCRv5_mobile_det_infer.onnx").is_file() {
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

#[tauri::command]
fn load_data(state: State<'_, AppState>, filename: String) -> Result<Option<Value>, String> {
    config::load_json(&state.data_dir, &filename)
}

#[tauri::command]
fn save_data(state: State<'_, AppState>, filename: String, data: Value) -> Result<(), String> {
    config::save_json(&state.data_dir, &filename, &data)
}

#[tauri::command]
fn migration_status(state: State<'_, AppState>) -> Result<MigrationReport, String> {
    state
        .migration_report
        .lock()
        .map(|report| report.clone())
        .map_err(|_| "Migration status is unavailable".to_owned())
}

#[tauri::command]
fn toggle_overlay(app: AppHandle) -> Result<bool, String> {
    windows::toggle_overlay(&app)
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
fn window_close(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    // Electron's close button set isQuitting and quit the app; the OS
    // title-bar close path instead hid to tray. Track that intent so the
    // CloseRequested handler below can mirror both behaviors.
    *state
        .is_quitting
        .lock()
        .map_err(|_| "Quit state is unavailable".to_owned())? = true;
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn lock_overlay(app: AppHandle) -> Result<(), String> {
    windows::lock_overlay(&app, true)
}

#[tauri::command]
fn unlock_overlay(app: AppHandle) -> Result<(), String> {
    windows::lock_overlay(&app, false)
}

#[tauri::command]
fn resize_overlay(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    windows::resize_overlay(&app, width, height)
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
fn show_toast(app: AppHandle, state: State<'_, AppState>, payload: Value) -> Result<(), String> {
    *state
        .last_toast
        .lock()
        .map_err(|_| "Toast state is unavailable".to_owned())? = Some(payload.clone());
    windows::show_toast(&app, payload)
}

#[tauri::command]
fn hide_toast(app: AppHandle) -> Result<(), String> {
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
fn execute_macro(payload: input::MacroPayload) {
    input::execute(payload);
}

#[tauri::command]
fn open_sponsor(app: AppHandle, state: State<'_, AppState>, url: String) -> Result<bool, String> {
    let url = if url.starts_with("https://") || url.starts_with("http://") {
        url
    } else {
        DEFAULT_SPONSOR_URL.to_owned()
    };
    state
        .window_payloads
        .lock()
        .map_err(|_| "Sponsor state is unavailable".to_owned())?
        .sponsor_url = url.clone();
    windows::open_sponsor(&app, url)?;
    Ok(true)
}

#[tauri::command]
fn close_sponsor_window(app: AppHandle) -> Result<(), String> {
    windows::close_window(&app, "sponsor")
}

#[tauri::command]
fn get_sponsor_url(state: State<'_, AppState>) -> Result<String, String> {
    state
        .window_payloads
        .lock()
        .map(|payloads| payloads.sponsor_url.clone())
        .map_err(|_| "Sponsor state is unavailable".to_owned())
}

#[tauri::command]
fn open_ocr_help(
    app: AppHandle,
    state: State<'_, AppState>,
    language: String,
) -> Result<bool, String> {
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
fn close_ocr_help_window(app: AppHandle) -> Result<(), String> {
    windows::close_window(&app, "ocr-help")
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
fn ocr_model_status(state: State<'_, AppState>) -> Result<ocr::ModelStatus, String> {
    ocr::verify_models_in(state.ocr_model_dir.clone())
}

#[tauri::command]
async fn recognize_ocr_region(
    app: AppHandle,
    region: ocr::OcrRegion,
) -> Result<ocr::OcrText, String> {
    let state = app.state::<AppState>();
    let model_dir = state.ocr_model_dir.clone();

    // Load the engine outside of spawn_blocking if needed, since State is not Send.
    // But we need to hold the lock during recognition. Instead, move the entire
    // OCR pipeline into spawn_blocking using a channel to get the result back.
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<ocr::OcrText, String> {
        let frame = ocr::capture_region(region)?;
        let state = app.state::<AppState>();
        let mut engine_slot = state
            .ocr_engine
            .lock()
            .map_err(|_| "OCR engine is unavailable".to_owned())?;
        if engine_slot.is_none() {
            *engine_slot = Some(ocr::OcrEngine::load_from_directory(model_dir)?);
        }
        engine_slot
            .as_mut()
            .expect("OCR engine initialized")
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
fn start_ocr_region_select(
    app: AppHandle,
    state: State<'_, AppState>,
    display_id: Option<u32>,
) -> Result<(), String> {
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
    windows::open_ocr_select(&app, &display)
}

#[tauri::command]
fn ocr_region_selected(
    app: AppHandle,
    state: State<'_, AppState>,
    region: OcrSelectionEvent,
) -> Result<(), String> {
    let display = state
        .ocr_selection_display
        .lock()
        .map_err(|_| "OCR selection state is unavailable".to_owned())?
        .clone()
        .ok_or_else(|| "OCR selection display is unavailable".to_owned())?;
    let normalized = normalize_ocr_selection(&display, region);
    app.emit_to("main", "ocr-region-selected", normalized)
        .map_err(|error| error.to_string())?;
    windows::close_window(&app, "ocr-select")
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
fn cancel_ocr_region_select(app: AppHandle) -> Result<(), String> {
    windows::close_window(&app, "ocr-select")
}

fn main() {
    tauri::Builder::default()
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
                ocr_model_dir,
                migration_report: Mutex::new(migration_report),
                ocr_engine: Mutex::new(None),
                ocr_selection_display: Mutex::new(None),
                window_payloads: Mutex::new(WindowPayloads {
                    sponsor_url: DEFAULT_SPONSOR_URL.to_owned(),
                    ocr_help_language: "zh".to_owned(),
                }),
                overlay_snapshot: Mutex::new(OverlaySnapshot {
                    settings: Value::Object(serde_json::Map::new()),
                    data: Value::Array(Vec::new()),
                    selection: 0,
                }),
                last_toast: Mutex::new(None),
                is_quitting: Mutex::new(false),
            });
            tray::create(&app.handle())?;
            hooks::start(app.handle().clone()).map_err(std::io::Error::other)?;
            windows::create_all_auxiliary_windows(&app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_data,
            save_data,
            migration_status,
            toggle_overlay,
            window_minimize,
            window_tray,
            window_close,
            lock_overlay,
            unlock_overlay,
            resize_overlay,
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
}
