use crate::ocr::OcrDisplay;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};
use windows::Win32::{Foundation::RECT, UI::WindowsAndMessaging::GetWindowRect};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPosition {
    pub x: i32,
    pub y: i32,
}

const TOAST_BASE_HEIGHT: f64 = 110.0;
const TOAST_MAX_HEIGHT: f64 = 260.0;

pub fn ensure_overlay_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window("overlay") {
        return Ok(window);
    }
    WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("overlay.html".into()))
        .title("HD2 Overlay")
        .inner_size(300.0, 550.0)
        .position(50.0, 50.0)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        // Match the Electron overlay's `focusable: false`. A focusable overlay
        // can steal the foreground from the game, causing SendInput events to
        // be delivered to the WebView instead of Helldivers II.
        .focusable(false)
        .focused(false)
        .visible(false)
        .build()
        .map_err(|error| error.to_string())
}

fn ensure_toast_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window("toast") {
        return Ok(window);
    }
    let window = WebviewWindowBuilder::new(app, "toast", WebviewUrl::App("toast.html".into()))
        .title("HD2 Notification")
        .inner_size(760.0, 110.0)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focusable(false)
        .focused(false)
        .visible(false)
        .build()
        .map_err(|error| error.to_string())?;
    if let Err(error) = window.set_ignore_cursor_events(true) {
        let _ = window.destroy();
        return Err(error.to_string());
    }
    Ok(window)
}

fn ensure_sponsor_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window("sponsor") {
        return Ok(window);
    }
    WebviewWindowBuilder::new(app, "sponsor", WebviewUrl::App("sponsor.html".into()))
        .title("感谢您的赞助")
        .inner_size(525.0, 675.0)
        .min_inner_size(525.0, 675.0)
        .max_inner_size(525.0, 675.0)
        .decorations(false)
        .resizable(false)
        .visible(false)
        .build()
        .map_err(|error| error.to_string())
}

fn ensure_ocr_help_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window("ocr-help") {
        return Ok(window);
    }
    WebviewWindowBuilder::new(app, "ocr-help", WebviewUrl::App("ocr-help.html".into()))
        .title("OCR Help")
        .inner_size(525.0, 675.0)
        .min_inner_size(525.0, 675.0)
        .max_inner_size(525.0, 675.0)
        .decorations(false)
        .resizable(false)
        .visible(false)
        .build()
        .map_err(|error| error.to_string())
}

fn create_ocr_select_window(
    app: &AppHandle,
    display: &OcrDisplay,
) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window("ocr-select") {
        window.destroy().map_err(|error| error.to_string())?;
    }
    let window =
        WebviewWindowBuilder::new(app, "ocr-select", WebviewUrl::App("ocr-select.html".into()))
            .title("OCR region selector")
            // `screenshots::DisplayInfo` reports desktop coordinates in physical pixels.
            // The builder accepts logical dimensions only; the exact physical geometry
            // is set immediately after construction while the window remains hidden.
            .inner_size(1.0, 1.0)
            .position(0.0, 0.0)
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .visible(false)
            .build()
            .map_err(|error| error.to_string())?;
    if let Err(error) = window.set_size(PhysicalSize::new(
        display.bounds.width,
        display.bounds.height,
    )) {
        let _ = window.destroy();
        return Err(error.to_string());
    }
    if let Err(error) =
        window.set_position(PhysicalPosition::new(display.bounds.x, display.bounds.y))
    {
        let _ = window.destroy();
        return Err(error.to_string());
    }
    Ok(window)
}

pub fn overlay_is_visible(app: &AppHandle) -> Result<bool, String> {
    app.get_webview_window("overlay")
        .map(|window| window.is_visible().map_err(|error| error.to_string()))
        .transpose()
        .map(|visible| visible.unwrap_or(false))
}

pub fn show_overlay(
    app: &AppHandle,
    saved_position: Option<OverlayPosition>,
) -> Result<(), String> {
    let window = ensure_overlay_window(app)?;
    if let Some(position) = saved_position {
        set_window_position(&window, position)?;
    }
    window.show().map_err(|error| error.to_string())
}

pub fn minimize_main(app: &AppHandle) -> Result<(), String> {
    main_window(app)?
        .minimize()
        .map_err(|error| error.to_string())
}

pub fn hide_main(app: &AppHandle) -> Result<(), String> {
    main_window(app)?.hide().map_err(|error| error.to_string())
}

pub fn lock_overlay(app: &AppHandle, locked: bool) -> Result<(), String> {
    let Some(window) = app.get_webview_window("overlay") else {
        return Ok(());
    };
    window
        .set_ignore_cursor_events(locked)
        .map_err(|error| error.to_string())?;
    let event = if locked {
        "overlay-locked"
    } else {
        "overlay-unlocked"
    };
    app.emit_to("overlay", event, ())
        .map_err(|error| error.to_string())
}

pub fn resize_overlay(app: &AppHandle, width: f64, height: f64) -> Result<(), String> {
    let width = if width.is_finite() {
        width.clamp(100.0, 1000.0)
    } else {
        300.0
    };
    let height = if height.is_finite() {
        height.clamp(100.0, 800.0)
    } else {
        550.0
    };
    let Some(window) = app.get_webview_window("overlay") else {
        return Ok(());
    };
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|error| error.to_string())
}

pub fn set_overlay_position(app: &AppHandle, position: OverlayPosition) -> Result<(), String> {
    validate_overlay_position(position)?;
    let Some(window) = app.get_webview_window("overlay") else {
        return Ok(());
    };
    set_window_position(&window, position)
}

fn set_window_position(window: &WebviewWindow, position: OverlayPosition) -> Result<(), String> {
    validate_overlay_position(position)?;
    window
        .set_position(PhysicalPosition::new(position.x, position.y))
        .map_err(|error| error.to_string())
}

fn validate_overlay_position(position: OverlayPosition) -> Result<(), String> {
    const MAX_ABSOLUTE_POSITION: i32 = 100_000;
    if position.x < -MAX_ABSOLUTE_POSITION
        || position.x > MAX_ABSOLUTE_POSITION
        || position.y < -MAX_ABSOLUTE_POSITION
        || position.y > MAX_ABSOLUTE_POSITION
    {
        return Err("Overlay position is outside the supported desktop range".to_owned());
    }
    Ok(())
}

pub fn get_overlay_position(app: &AppHandle) -> Result<Option<OverlayPosition>, String> {
    let Some(window) = app.get_webview_window("overlay") else {
        return Ok(None);
    };
    let mut rect = RECT::default();
    // SAFETY: `window` keeps the WebView window object alive while `hwnd()` is
    // obtained, and `rect` is a valid writable RECT for the entire call.
    // GetWindowRect writes synchronously and does not retain either value.
    unsafe { GetWindowRect(window.hwnd().map_err(|error| error.to_string())?, &mut rect) }
        .map_err(|error| error.to_string())?;
    Ok(Some(OverlayPosition {
        x: rect.left,
        y: rect.top,
    }))
}

pub fn emit_overlay(
    app: &AppHandle,
    event: &str,
    payload: serde_json::Value,
) -> Result<(), String> {
    if app.get_webview_window("overlay").is_some() {
        app.emit_to("overlay", event, payload)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn open_sponsor(app: &AppHandle, url: String) -> Result<(), String> {
    let window = ensure_sponsor_window(app)?;
    show_fixed_window(app, &window, "sponsor", "sponsor-url", url)
}

pub fn open_ocr_help(app: &AppHandle, language: String) -> Result<(), String> {
    let window = ensure_ocr_help_window(app)?;
    show_fixed_window(app, &window, "ocr-help", "ocr-help-lang", language)
}

pub fn open_ocr_select(app: &AppHandle, display: &OcrDisplay) -> Result<(), String> {
    let window = create_ocr_select_window(app, display)?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(())
}

pub fn destroy_window(app: &AppHandle, label: &str) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(label) {
        window.destroy().map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn show_toast(app: &AppHandle, payload: serde_json::Value) -> Result<(), String> {
    let window = ensure_toast_window(app)?;
    position_toast_window(&window, &payload)?;
    app.emit_to("toast", "show-toast", payload)
        .map_err(|error| error.to_string())?;
    window.show().map_err(|error| error.to_string())?;
    Ok(())
}

fn position_toast_window(
    window: &WebviewWindow,
    payload: &serde_json::Value,
) -> Result<(), String> {
    let displays = crate::capture::displays()?;
    let display = displays
        .iter()
        .find(|display| display.is_primary)
        .or_else(|| displays.first())
        .copied()
        .ok_or_else(|| "Cannot find the primary display for toast".to_owned())?;
    let (x, y, width, height) = toast_geometry(display, payload);
    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|error| error.to_string())?;
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| error.to_string())
}

fn toast_geometry(
    display: crate::capture::Display,
    payload: &serde_json::Value,
) -> (i32, i32, u32, u32) {
    let scale = if display.scale_factor.is_finite() {
        display.scale_factor.clamp(0.5, 8.0)
    } else {
        1.0
    };
    let scaled = |logical: f64| (logical * f64::from(scale)).round().max(1.0) as u32;
    let available_width = display.work_width.saturating_sub(scaled(80.0)).max(1);
    let minimum_width = scaled(120.0).min(display.work_width);
    let width = available_width.min(scaled(980.0)).max(minimum_width);
    let height = scaled(toast_height(payload)).min(display.work_height);
    let x = display.work_x + (display.work_width.saturating_sub(width) / 2) as i32;
    let y = display.work_y
        + display
            .work_height
            .saturating_sub(height)
            .saturating_sub(scaled(90.0)) as i32;
    (x, y, width, height)
}

fn toast_height(payload: &serde_json::Value) -> f64 {
    if !payload
        .get("isError")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return TOAST_BASE_HEIGHT;
    }
    let message_length = payload
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(|message| message.chars().count())
        .unwrap_or(0);
    let estimated_lines = (message_length as f64 / 92.0).ceil().max(2.0);
    (58.0 + estimated_lines * 24.0).clamp(TOAST_BASE_HEIGHT, TOAST_MAX_HEIGHT)
}

pub fn hide_toast(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("toast") {
        // Toasts are infrequent and already restore their latest payload when
        // recreated. Destroying the transparent WebView releases its renderer
        // and GPU composition resources instead of retaining them while idle.
        window.destroy().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn show_fixed_window(
    app: &AppHandle,
    window: &WebviewWindow,
    label: &str,
    event: &str,
    payload: String,
) -> Result<(), String> {
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    app.emit_to(label, event, payload)
        .map_err(|error| error.to_string())
}

fn main_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "Main window is unavailable".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_legacy_toast_height_rules() {
        assert_eq!(
            toast_height(&serde_json::json!({ "message": "normal" })),
            110.0
        );
        assert_eq!(
            toast_height(&serde_json::json!({ "isError": true, "message": "x" })),
            110.0
        );
        assert_eq!(
            toast_height(&serde_json::json!({ "isError": true, "message": "x".repeat(500) })),
            202.0
        );
    }

    #[test]
    fn rejects_unreasonable_overlay_positions() {
        assert!(validate_overlay_position(OverlayPosition { x: 50, y: -240 }).is_ok());
        assert!(validate_overlay_position(OverlayPosition { x: 100_001, y: 0 }).is_err());
        assert!(validate_overlay_position(OverlayPosition { x: 0, y: -100_001 }).is_err());
    }

    #[test]
    fn toast_geometry_scales_css_height_and_uses_the_work_area() {
        let display = crate::capture::Display {
            id: 1,
            x: 0,
            y: 0,
            width: 3840,
            height: 2160,
            work_x: 0,
            work_y: 0,
            work_width: 3840,
            work_height: 2080,
            scale_factor: 2.0,
            is_primary: true,
        };
        assert_eq!(
            toast_geometry(display, &serde_json::json!({ "message": "normal" })),
            (940, 1680, 1960, 220)
        );
    }
}
