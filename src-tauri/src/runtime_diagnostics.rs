use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_RECENT_INCIDENTS: usize = 32;
const MAX_IDENTIFIER_LENGTH: usize = 48;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeIncident {
    pub occurred_at_unix_ms: u64,
    pub severity: &'static str,
    pub component: String,
    pub operation: String,
    pub code: String,
    pub occurrences: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingDiagnostics {
    pub active: bool,
    pub attempts_started: u64,
    pub inputs_observed: u64,
    pub attempts_completed: u64,
    pub attempts_failed: u64,
    pub cancellations_without_input: u64,
    pub current_attempt_has_input: bool,
    pub current_attempt_age_ms: Option<u64>,
    pub last_stage: Option<String>,
    pub last_event_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiagnostics {
    pub error_count: u64,
    pub warning_count: u64,
    pub recent_incidents: Vec<RuntimeIncident>,
    pub binding: BindingDiagnostics,
}

#[derive(Default)]
struct RuntimeState {
    error_count: u64,
    warning_count: u64,
    recent_incidents: VecDeque<RuntimeIncident>,
    binding: BindingState,
}

#[derive(Default)]
struct BindingState {
    active: bool,
    attempts_started: u64,
    inputs_observed: u64,
    attempts_completed: u64,
    attempts_failed: u64,
    cancellations_without_input: u64,
    current_attempt_has_input: bool,
    current_attempt_started_at_unix_ms: Option<u64>,
    last_stage: Option<String>,
    last_event_at_unix_ms: Option<u64>,
}

impl BindingState {
    fn record_stage(&mut self, stage: &str, now: u64) {
        match stage {
            "started" => {
                self.active = true;
                self.attempts_started = self.attempts_started.saturating_add(1);
                self.current_attempt_has_input = false;
                self.current_attempt_started_at_unix_ms = Some(now);
            }
            "input_observed" => {
                if self.active && !self.current_attempt_has_input {
                    self.inputs_observed = self.inputs_observed.saturating_add(1);
                }
                self.current_attempt_has_input = true;
            }
            "completed" => {
                self.active = false;
                self.attempts_completed = self.attempts_completed.saturating_add(1);
                self.current_attempt_started_at_unix_ms = None;
            }
            "cancelled" => {
                if self.active && !self.current_attempt_has_input {
                    self.cancellations_without_input =
                        self.cancellations_without_input.saturating_add(1);
                }
                self.active = false;
                self.current_attempt_started_at_unix_ms = None;
            }
            "failed" | "filter_failed" => {
                self.active = false;
                self.attempts_failed = self.attempts_failed.saturating_add(1);
                self.current_attempt_started_at_unix_ms = None;
            }
            _ => unreachable!(),
        }
        self.last_stage = Some(stage.to_owned());
        self.last_event_at_unix_ms = Some(now);
    }
}

static RUNTIME_STATE: OnceLock<Mutex<RuntimeState>> = OnceLock::new();

fn runtime_state() -> &'static Mutex<RuntimeState> {
    RUNTIME_STATE.get_or_init(|| Mutex::new(RuntimeState::default()))
}

pub fn record_error(component: &str, operation: &str, code: &str) {
    record_incident("error", component, operation, code);
}

pub fn record_warning(component: &str, operation: &str, code: &str) {
    record_incident("warning", component, operation, code);
}

fn record_incident(severity: &'static str, component: &str, operation: &str, code: &str) {
    let occurred_at_unix_ms = unix_time_millis();
    let component = allowlisted_component(component).to_owned();
    let operation = allowlisted_operation(operation).to_owned();
    let code = allowlisted_code(code).to_owned();
    let mut state = runtime_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if severity == "error" {
        state.error_count = state.error_count.saturating_add(1);
    } else {
        state.warning_count = state.warning_count.saturating_add(1);
    }

    if let Some(existing) = state.recent_incidents.back_mut().filter(|incident| {
        incident.severity == severity
            && incident.component == component
            && incident.operation == operation
            && incident.code == code
    }) {
        existing.occurred_at_unix_ms = occurred_at_unix_ms;
        existing.occurrences = existing.occurrences.saturating_add(1);
        return;
    }
    if state.recent_incidents.len() == MAX_RECENT_INCIDENTS {
        state.recent_incidents.pop_front();
    }
    state.recent_incidents.push_back(RuntimeIncident {
        occurred_at_unix_ms,
        severity,
        component,
        operation,
        code,
        occurrences: 1,
    });
}

pub fn record_binding_stage(stage: &str) -> Result<(), String> {
    let stage = safe_identifier(stage);
    if !matches!(
        stage.as_str(),
        "started" | "input_observed" | "completed" | "cancelled" | "failed" | "filter_failed"
    ) {
        return Err("Unsupported binding diagnostic stage".to_owned());
    }

    let now = unix_time_millis();
    let mut state = runtime_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.binding.record_stage(&stage, now);
    Ok(())
}

pub fn snapshot() -> RuntimeDiagnostics {
    let now = unix_time_millis();
    let state = runtime_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    RuntimeDiagnostics {
        error_count: state.error_count,
        warning_count: state.warning_count,
        recent_incidents: state.recent_incidents.iter().cloned().collect(),
        binding: BindingDiagnostics {
            active: state.binding.active,
            attempts_started: state.binding.attempts_started,
            inputs_observed: state.binding.inputs_observed,
            attempts_completed: state.binding.attempts_completed,
            attempts_failed: state.binding.attempts_failed,
            cancellations_without_input: state.binding.cancellations_without_input,
            current_attempt_has_input: state.binding.current_attempt_has_input,
            current_attempt_age_ms: state
                .binding
                .active
                .then_some(state.binding.current_attempt_started_at_unix_ms)
                .flatten()
                .map(|started| now.saturating_sub(started)),
            last_stage: state.binding.last_stage.clone(),
            last_event_at_unix_ms: state.binding.last_event_at_unix_ms,
        },
    }
}

fn safe_identifier(value: &str) -> String {
    let normalized = value
        .chars()
        .filter_map(|character| {
            let character = character.to_ascii_lowercase();
            (character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
                .then_some(character)
        })
        .take(MAX_IDENTIFIER_LENGTH)
        .collect::<String>();
    if normalized.is_empty() {
        "unknown".to_owned()
    } else {
        normalized
    }
}

fn allowlisted_component(value: &str) -> &'static str {
    match safe_identifier(value).as_str() {
        "frontend" => "frontend",
        "input" => "input",
        _ => "unknown_component",
    }
}

fn allowlisted_operation(value: &str) -> &'static str {
    match safe_identifier(value).as_str() {
        "shortcut_binding" => "shortcut_binding",
        "shortcut_filter_update" => "shortcut_filter_update",
        "macro_start" => "macro_start",
        "macro_execution" => "macro_execution",
        "binding_event_delivery" => "binding_event_delivery",
        "shortcut_event_delivery" => "shortcut_event_delivery",
        "macro_event_delivery" => "macro_event_delivery",
        "ocr_auto_equip" => "ocr_auto_equip",
        "renderer_runtime" => "renderer_runtime",
        "window_minimize" => "window_minimize",
        "window_tray" => "window_tray",
        "begin_exit" => "begin_exit",
        "window_close" => "window_close",
        "execute_macro" => "execute_macro",
        "set_global_input_filter" => "set_global_input_filter",
        "get_input_diagnostics" => "get_input_diagnostics",
        "collect_diagnostics_report" => "collect_diagnostics_report",
        "export_diagnostics_report" => "export_diagnostics_report",
        "toggle_overlay" => "toggle_overlay",
        "lock_overlay" => "lock_overlay",
        "unlock_overlay" => "unlock_overlay",
        "resize_overlay" => "resize_overlay",
        "set_overlay_position" => "set_overlay_position",
        "update_overlay_settings" => "update_overlay_settings",
        "update_overlay" => "update_overlay",
        "highlight_overlay" => "highlight_overlay",
        "update_selection" => "update_selection",
        "get_overlay_snapshot" => "get_overlay_snapshot",
        "load_data" => "load_data",
        "save_data" => "save_data",
        "show_toast" => "show_toast",
        "hide_toast" => "hide_toast",
        "get_last_toast" => "get_last_toast",
        "open_sponsor" => "open_sponsor",
        "close_sponsor_window" => "close_sponsor_window",
        "get_sponsor_url" => "get_sponsor_url",
        "open_ocr_help" => "open_ocr_help",
        "close_ocr_help_window" => "close_ocr_help_window",
        "get_ocr_help_language" => "get_ocr_help_language",
        "get_app_version" => "get_app_version",
        "check_for_updates" => "check_for_updates",
        "open_release_download" => "open_release_download",
        "get_ocr_displays" => "get_ocr_displays",
        "start_ocr_region_select" => "start_ocr_region_select",
        "ocr_region_selected" => "ocr_region_selected",
        "cancel_ocr_region_select" => "cancel_ocr_region_select",
        "recognize_ocr_region" => "recognize_ocr_region",
        _ => "unknown_operation",
    }
}

fn allowlisted_code(value: &str) -> &'static str {
    match safe_identifier(value).as_str() {
        "timeout" => "timeout",
        "permission_denied" => "permission_denied",
        "invalid_data" => "invalid_data",
        "unavailable" => "unavailable",
        "cancelled" => "cancelled",
        "ipc_rejected" => "ipc_rejected",
        "configuration_rejected" => "configuration_rejected",
        "macro_already_running" => "macro_already_running",
        "windows_input_rejected" => "windows_input_rejected",
        "input_cleanup_failed" => "input_cleanup_failed",
        "macro_cancelled" => "macro_cancelled",
        "macro_execution_failed" => "macro_execution_failed",
        "event_delivery_failed" => "event_delivery_failed",
        "empty_chord_confirmation" => "empty_chord_confirmation",
        "chord_limit_exceeded" => "chord_limit_exceeded",
        "ocr_hotkey_conflict" => "ocr_hotkey_conflict",
        "region_not_configured" => "region_not_configured",
        "ipc_unavailable" => "ipc_unavailable",
        "no_text_detected" => "no_text_detected",
        "no_stratagem_match" => "no_stratagem_match",
        "no_available_slot" => "no_available_slot",
        "uncaught_error" => "uncaught_error",
        "unhandled_rejection" => "unhandled_rejection",
        _ => "unknown_failure",
    }
}

fn unix_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_cannot_leak_free_form_values() {
        assert_eq!(
            safe_identifier("Set Global/Input Filter: F8"),
            "setglobalinputfilterf8"
        );
        assert_eq!(safe_identifier("路径/secret"), "secret");
        assert_eq!(safe_identifier(""), "unknown");
        assert_eq!(allowlisted_operation("ControlLeft+F8"), "unknown_operation");
        assert_eq!(allowlisted_code("secret stratagem"), "unknown_failure");
    }

    #[test]
    fn binding_state_can_represent_a_cancelled_attempt_without_input() {
        let mut state = BindingState::default();
        state.record_stage("started", 100);
        state.record_stage("cancelled", 200);
        assert_eq!(state.attempts_started, 1);
        assert_eq!(state.cancellations_without_input, 1);
        assert!(!state.active);
    }

    #[test]
    fn binding_state_records_successful_input_and_completion() {
        let mut state = BindingState::default();
        state.record_stage("started", 100);
        state.record_stage("input_observed", 150);
        state.record_stage("input_observed", 160);
        state.record_stage("completed", 200);
        assert_eq!(state.inputs_observed, 1);
        assert_eq!(state.attempts_completed, 1);
        assert_eq!(state.cancellations_without_input, 0);
        assert_eq!(state.last_stage.as_deref(), Some("completed"));
    }
}
