use crate::ocr::OcrDisplay;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

const TOAST_BASE_HEIGHT: f64 = 110.0;
const TOAST_MAX_HEIGHT: f64 = 260.0;

pub fn create_all_auxiliary_windows(app: &AppHandle) {
    let _ = create_overlay_window(app);
    let _ = create_toast_window(app);
    let _ = create_sponsor_window(app);
    let _ = create_ocr_help_window(app);
    let _ = create_ocr_select_window(app);
}

fn create_overlay_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if app.get_webview_window("overlay").is_some() {
        return Ok(app.get_webview_window("overlay").unwrap());
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
        .focused(false)
        .visible(false)
        .build()
        .map_err(|error| error.to_string())
}

fn create_toast_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if app.get_webview_window("toast").is_some() {
        return Ok(app.get_webview_window("toast").unwrap());
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
        .focused(false)
        .visible(false)
        .build()
        .map_err(|error| error.to_string())?;
    window
        .set_ignore_cursor_events(true)
        .map_err(|error| error.to_string())?;
    Ok(window)
}

fn create_sponsor_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if app.get_webview_window("sponsor").is_some() {
        return Ok(app.get_webview_window("sponsor").unwrap());
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

fn create_ocr_help_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if app.get_webview_window("ocr-help").is_some() {
        return Ok(app.get_webview_window("ocr-help").unwrap());
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

fn create_ocr_select_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if app.get_webview_window("ocr-select").is_some() {
        return Ok(app.get_webview_window("ocr-select").unwrap());
    }
    let display = screenshots::Screen::all()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|screen| screen.display_info)
        .find(|display| display.is_primary)
        .ok_or_else(|| "Cannot find the primary display".to_owned())?;
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
    window
        .set_size(PhysicalSize::new(display.width, display.height))
        .map_err(|error| error.to_string())?;
    window
        .set_position(PhysicalPosition::new(display.x, display.y))
        .map_err(|error| error.to_string())?;
    Ok(window)
}

pub fn toggle_overlay(app: &AppHandle) -> Result<bool, String> {
    let window = app
        .get_webview_window("overlay")
        .ok_or_else(|| "Overlay window is unavailable".to_owned())?;
    let visible = window.is_visible().map_err(|error| error.to_string())?;
    if visible {
        window.hide().map_err(|error| error.to_string())?;
    } else {
        window.show().map_err(|error| error.to_string())?;
    }
    Ok(!visible)
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
    let width = width.max(50.0);
    let height = height.max(50.0);
    let Some(window) = app.get_webview_window("overlay") else {
        return Ok(());
    };
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|error| error.to_string())
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
    show_fixed_window(app, "sponsor", "sponsor-url", url)
}

pub fn open_ocr_help(app: &AppHandle, language: String) -> Result<(), String> {
    show_fixed_window(app, "ocr-help", "ocr-help-lang", language)
}

pub fn open_ocr_select(app: &AppHandle, display: &OcrDisplay) -> Result<(), String> {
    let window = app
        .get_webview_window("ocr-select")
        .ok_or_else(|| "OCR select window is unavailable".to_owned())?;
    window
        .set_size(PhysicalSize::new(
            display.bounds.width,
            display.bounds.height,
        ))
        .map_err(|error| error.to_string())?;
    window
        .set_position(PhysicalPosition::new(display.bounds.x, display.bounds.y))
        .map_err(|error| error.to_string())?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(())
}

pub fn close_window(app: &AppHandle, label: &str) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(label) {
        window.hide().map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn show_toast(app: &AppHandle, payload: serde_json::Value) -> Result<(), String> {
    let window = app
        .get_webview_window("toast")
        .ok_or_else(|| "Toast window is unavailable".to_owned())?;
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
    let display = screenshots::Screen::all()
        .map_err(|error| format!("Cannot find the primary display for toast: {error}"))?
        .into_iter()
        .map(|screen| screen.display_info)
        .find(|display| display.is_primary)
        .ok_or_else(|| "Cannot find the primary display for toast".to_owned())?;
    let width = (display.width.saturating_sub(80) as f64).clamp(120.0, 980.0);
    let height = toast_height(payload);
    let x = display.x as f64 + (display.width as f64 - width) / 2.0;
    let y = display.y as f64 + display.height as f64 - height - 90.0;
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|error| error.to_string())?;
    window
        .set_position(LogicalPosition::new(x, y))
        .map_err(|error| error.to_string())
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
    let estimated_lines = ((message_length as f64 / 92.0).ceil() as f64).max(2.0);
    (58.0 + estimated_lines * 24.0).clamp(TOAST_BASE_HEIGHT, TOAST_MAX_HEIGHT)
}

pub fn hide_toast(app: &AppHandle) -> Result<(), String> {
    app.get_webview_window("toast")
        .ok_or_else(|| "Toast window is unavailable".to_owned())?
        .hide()
        .map_err(|error| error.to_string())
}

fn show_fixed_window(
    app: &AppHandle,
    label: &str,
    event: &str,
    payload: String,
) -> Result<(), String> {
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| format!("{} window is unavailable", label))?;
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
}
