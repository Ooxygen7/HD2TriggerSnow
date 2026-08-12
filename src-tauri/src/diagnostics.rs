use crate::{config, hooks, ocr, runtime_diagnostics, updates};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_EXPORTED_REPORT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationDiagnostics {
    pub version: String,
    pub build_profile: &'static str,
    pub operating_system: &'static str,
    pub architecture: &'static str,
    pub webview_version: Option<String>,
    pub process_id: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowDiagnostics {
    pub main_window_exists: bool,
    pub main_window_visible: bool,
    pub overlay_window_exists: bool,
    pub overlay_window_visible: bool,
    pub overlay_locked: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiagnostics {
    pub name: &'static str,
    pub status: &'static str,
    pub size_bytes: Option<u64>,
    pub entry_count: Option<usize>,
    pub backup_available: bool,
    pub issue: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDiagnostics {
    pub directory_ready: bool,
    pub writable: bool,
    pub invalid_file_count: usize,
    pub recoverable_backup_count: usize,
    pub files: Vec<FileDiagnostics>,
    pub issue: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationDiagnostics {
    pub settings_loaded: bool,
    pub slot_count: usize,
    pub equipped_stratagems: usize,
    pub bound_stratagems: usize,
    pub preset_count: usize,
    pub custom_stratagem_count: usize,
    pub duplicate_binding_groups: usize,
    pub ocr_region_configured: bool,
    pub ocr_display_configured: bool,
    pub direction_only: bool,
    pub toast_notifications: bool,
    pub auto_open_overlay: bool,
    pub auto_lock_overlay: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaticOcrDiagnostics {
    pub model_files_present: bool,
    pub self_test: DiagnosticResult<ocr::ModelStatus>,
    pub displays: DiagnosticResult<DisplayDiagnostics>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayDiagnostics {
    pub display_count: usize,
    pub primary_width: Option<u32>,
    pub primary_height: Option<u32>,
    pub primary_scale_factor: Option<f32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticResult<T> {
    pub ok: bool,
    pub value: Option<T>,
    pub error: Option<String>,
}

impl<T> DiagnosticResult<T> {
    pub fn success(value: T) -> Self {
        Self {
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    pub fn failure(error: String) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(error),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDiagnostics {
    pub application: ApplicationDiagnostics,
    pub windows: WindowDiagnostics,
    pub storage: StorageDiagnostics,
    pub configuration: ConfigurationDiagnostics,
    pub migration: config::MigrationReport,
    pub input: hooks::InputDiagnostics,
    pub runtime: runtime_diagnostics::RuntimeDiagnostics,
    pub ocr_model_files_present: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    pub schema_version: u8,
    pub generated_at_unix_ms: u128,
    pub application: ApplicationDiagnostics,
    pub windows: WindowDiagnostics,
    pub storage: StorageDiagnostics,
    pub configuration: ConfigurationDiagnostics,
    pub migration: config::MigrationReport,
    pub input: hooks::InputDiagnostics,
    pub runtime: runtime_diagnostics::RuntimeDiagnostics,
    pub ocr: StaticOcrDiagnostics,
    pub update_service: updates::UpdateEndpointDiagnostics,
    pub privacy: PrivacyDiagnostics,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyDiagnostics {
    pub includes_user_names: bool,
    pub includes_full_paths: bool,
    pub includes_hotkey_values: bool,
    pub includes_stratagem_names: bool,
    pub includes_screenshots: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub filename: String,
}

pub fn collect_local(
    data_dir: &Path,
    ocr_model_dir: &Path,
    migration: config::MigrationReport,
    windows: WindowDiagnostics,
) -> LocalDiagnostics {
    let (storage, values) = inspect_storage(data_dir);
    LocalDiagnostics {
        application: ApplicationDiagnostics {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            build_profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            operating_system: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            webview_version: tauri::webview_version().ok(),
            process_id: std::process::id(),
        },
        windows,
        configuration: summarize_configuration(&values),
        storage,
        migration,
        input: hooks::diagnostics(),
        runtime: runtime_diagnostics::snapshot(),
        ocr_model_files_present: ocr::model_files_exist(ocr_model_dir),
    }
}

pub fn assemble_report(
    local: LocalDiagnostics,
    ocr_self_test: Result<ocr::ModelStatus, String>,
    displays: Result<Vec<ocr::OcrDisplay>, String>,
    update_service: updates::UpdateEndpointDiagnostics,
    redacted_roots: &[PathBuf],
) -> DiagnosticReport {
    let ocr_self_test = match ocr_self_test {
        Ok(status) => DiagnosticResult::success(status),
        Err(error) => DiagnosticResult::failure(sanitize_error(&error, redacted_roots)),
    };
    let displays = match displays {
        Ok(displays) => {
            let primary = displays
                .iter()
                .find(|display| display.is_primary)
                .or_else(|| displays.first());
            DiagnosticResult::success(DisplayDiagnostics {
                display_count: displays.len(),
                primary_width: primary.map(|display| display.bounds.width),
                primary_height: primary.map(|display| display.bounds.height),
                primary_scale_factor: primary.map(|display| display.scale_factor),
            })
        }
        Err(error) => DiagnosticResult::failure(sanitize_error(&error, redacted_roots)),
    };
    DiagnosticReport {
        schema_version: 1,
        generated_at_unix_ms: unix_time_millis(),
        application: local.application,
        windows: local.windows,
        storage: local.storage,
        configuration: local.configuration,
        migration: local.migration,
        input: local.input,
        runtime: local.runtime,
        ocr: StaticOcrDiagnostics {
            model_files_present: local.ocr_model_files_present,
            self_test: ocr_self_test,
            displays,
        },
        update_service,
        privacy: PrivacyDiagnostics {
            includes_user_names: false,
            includes_full_paths: false,
            includes_hotkey_values: false,
            includes_stratagem_names: false,
            includes_screenshots: false,
        },
    }
}

pub fn export_report(download_dir: &Path, report: &Value) -> Result<ExportResult, String> {
    if report.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err("Unsupported diagnostics report schema".to_owned());
    }
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| format!("Cannot serialize diagnostics report: {error}"))?;
    if bytes.len() > MAX_EXPORTED_REPORT_BYTES {
        return Err("Diagnostics report exceeds the 256 KiB limit".to_owned());
    }
    fs::create_dir_all(download_dir)
        .map_err(|error| format!("Cannot prepare the Downloads folder: {error}"))?;
    let timestamp = unix_time_millis();
    let (filename, path, mut file) = (0_u8..100)
        .find_map(|suffix| {
            let filename = if suffix == 0 {
                format!("HD2-Diagnostics-{timestamp}.json")
            } else {
                format!("HD2-Diagnostics-{timestamp}-{suffix}.json")
            };
            let path = download_dir.join(&filename);
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(file) => Some(Ok((filename, path, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .transpose()
        .map_err(|error| format!("Cannot create the diagnostics report: {error}"))?
        .ok_or_else(|| "Cannot allocate a unique diagnostics report filename".to_owned())?;
    if let Err(error) = file
        .write_all(&bytes)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(format!("Cannot finish the diagnostics report: {error}"));
    }
    Ok(ExportResult { filename })
}

fn inspect_storage(data_dir: &Path) -> (StorageDiagnostics, HashMap<&'static str, Value>) {
    let directory_ready = data_dir.is_dir();
    let (writable, issue) = test_directory_writable(data_dir);
    let mut values = HashMap::new();
    let mut files = Vec::with_capacity(config::LEGACY_FILES.len());
    for filename in config::LEGACY_FILES {
        let path = data_dir.join(filename);
        let backup_available = path.with_extension("json.backup").is_file();
        if !path.is_file() {
            files.push(FileDiagnostics {
                name: filename,
                status: "missing",
                size_bytes: None,
                entry_count: None,
                backup_available,
                issue: None,
            });
            continue;
        }
        let size_bytes = fs::metadata(&path).ok().map(|metadata| metadata.len());
        match config::load_json(data_dir, filename) {
            Ok(Some(value)) => {
                let entry_count = match &value {
                    Value::Array(items) => Some(items.len()),
                    Value::Object(items) => Some(items.len()),
                    _ => None,
                };
                values.insert(filename, value);
                files.push(FileDiagnostics {
                    name: filename,
                    status: "valid",
                    size_bytes,
                    entry_count,
                    backup_available,
                    issue: None,
                });
            }
            Ok(None) => unreachable!("the file existence check already succeeded"),
            Err(error) => files.push(FileDiagnostics {
                name: filename,
                status: "invalid",
                size_bytes,
                entry_count: None,
                backup_available,
                issue: Some(sanitize_error(&error, &[data_dir.to_owned()])),
            }),
        }
    }
    let invalid_file_count = files.iter().filter(|file| file.status == "invalid").count();
    let recoverable_backup_count = files
        .iter()
        .filter(|file| file.status == "invalid" && file.backup_available)
        .count();
    (
        StorageDiagnostics {
            directory_ready,
            writable,
            invalid_file_count,
            recoverable_backup_count,
            files,
            issue,
        },
        values,
    )
}

fn summarize_configuration(values: &HashMap<&'static str, Value>) -> ConfigurationDiagnostics {
    let settings = values.get("settings.json").and_then(Value::as_object);
    let loadout = values.get("loadout.json").and_then(Value::as_array);
    let presets = values.get("presets.json").and_then(Value::as_array);
    let custom = values.get("custom_strats.json").and_then(Value::as_array);
    let mut binding_counts = HashMap::<&str, usize>::new();
    let mut equipped_stratagems = 0;
    let mut bound_stratagems = 0;
    if let Some(loadout) = loadout {
        for item in loadout.iter().filter_map(Value::as_object) {
            if item.get("stratId").is_some_and(|value| !value.is_null()) {
                equipped_stratagems += 1;
            }
            if let Some(binding) = item
                .get("hotkey")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|binding| !binding.is_empty())
            {
                bound_stratagems += 1;
                *binding_counts.entry(binding).or_default() += 1;
            }
        }
    }
    if let Some(binding) = settings
        .and_then(|settings| settings.get("ocrHotkey"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|binding| !binding.is_empty())
    {
        *binding_counts.entry(binding).or_default() += 1;
    }
    ConfigurationDiagnostics {
        settings_loaded: settings.is_some(),
        slot_count: settings
            .and_then(|settings| settings.get("slotCount"))
            .and_then(Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .or_else(|| loadout.map(Vec::len))
            .unwrap_or(0),
        equipped_stratagems,
        bound_stratagems,
        preset_count: presets.map(Vec::len).unwrap_or(0),
        custom_stratagem_count: custom.map(Vec::len).unwrap_or(0),
        duplicate_binding_groups: binding_counts.values().filter(|count| **count > 1).count(),
        ocr_region_configured: settings
            .and_then(|settings| settings.get("ocrRegion"))
            .is_some_and(|value| value.is_object()),
        ocr_display_configured: settings
            .and_then(|settings| settings.get("ocrDisplayId"))
            .is_some_and(|value| !value.is_null()),
        direction_only: setting_bool(settings, "directionOnly", false),
        toast_notifications: setting_bool(settings, "showToasts", true),
        auto_open_overlay: setting_bool(settings, "autoOpenOverlay", false),
        auto_lock_overlay: setting_bool(settings, "autoLockOverlay", false),
    }
}

fn setting_bool(
    settings: Option<&serde_json::Map<String, Value>>,
    key: &str,
    fallback: bool,
) -> bool {
    settings
        .and_then(|settings| settings.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(fallback)
}

fn test_directory_writable(data_dir: &Path) -> (bool, Option<String>) {
    if !data_dir.is_dir() {
        return (
            false,
            Some("Application data directory is missing".to_owned()),
        );
    }
    let probe = data_dir.join(format!(
        ".diagnostics-write-probe-{}-{}",
        std::process::id(),
        unix_time_nanos()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&probe)?;
        file.write_all(b"diagnostics")?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::remove_file(&probe)
    })();
    match result {
        Ok(()) => (true, None),
        Err(error) => {
            let _ = fs::remove_file(probe);
            (
                false,
                Some(sanitize_error(
                    &format!("Application data directory is not writable: {error}"),
                    &[data_dir.to_owned()],
                )),
            )
        }
    }
}

pub fn sanitize_error(error: &str, redacted_roots: &[PathBuf]) -> String {
    let mut sanitized = error.replace(['\r', '\n', '\t'], " ");
    for root in redacted_roots {
        let rendered = root.to_string_lossy();
        if !rendered.is_empty() {
            sanitized = sanitized.replace(rendered.as_ref(), "<redacted-path>");
        }
    }
    sanitized.chars().take(320).collect()
}

fn unix_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn unix_time_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hd2-diagnostics-{label}-{}-{}",
            std::process::id(),
            unix_time_millis()
        ));
        fs::create_dir_all(&path).expect("temporary directory should be creatable");
        path
    }

    #[test]
    fn storage_report_contains_statuses_but_not_file_contents_or_paths() {
        let directory = temporary_directory("storage");
        fs::write(
            directory.join("settings.json"),
            br#"{"menuKey":"SecretKey","directionOnly":true}"#,
        )
        .expect("settings should be writable");
        fs::write(directory.join("loadout.json"), b"{broken")
            .expect("invalid loadout should be writable");

        let (report, _) = inspect_storage(&directory);
        let serialized = serde_json::to_string(&report).expect("report should serialize");
        assert!(report.writable);
        assert_eq!(report.invalid_file_count, 1);
        assert!(!serialized.contains("SecretKey"));
        assert!(!serialized.contains(directory.to_string_lossy().as_ref()));
        fs::remove_dir_all(directory).expect("temporary directory should be removable");
    }

    #[test]
    fn configuration_summary_counts_bindings_without_exporting_values() {
        let mut values = HashMap::new();
        values.insert(
            "settings.json",
            serde_json::json!({
                "slotCount": 4,
                "ocrHotkey": "F8",
                "directionOnly": true,
                "ocrRegion": {"x": 10, "y": 20, "width": 30, "height": 40}
            }),
        );
        values.insert(
            "loadout.json",
            serde_json::json!([
                {"stratId": "secret-one", "hotkey": "F8"},
                {"stratId": "secret-two", "hotkey": "F9"}
            ]),
        );

        let summary = summarize_configuration(&values);
        let serialized = serde_json::to_string(&summary).expect("summary should serialize");
        assert_eq!(summary.equipped_stratagems, 2);
        assert_eq!(summary.bound_stratagems, 2);
        assert_eq!(summary.duplicate_binding_groups, 1);
        assert!(summary.direction_only);
        assert!(summary.ocr_region_configured);
        assert!(!serialized.contains("secret-one"));
        assert!(!serialized.contains("F8"));
    }

    #[test]
    fn export_rejects_unknown_or_oversized_reports() {
        let directory = temporary_directory("export");
        assert!(export_report(&directory, &serde_json::json!({"schemaVersion": 2})).is_err());
        let oversized = "x".repeat(MAX_EXPORTED_REPORT_BYTES);
        assert!(export_report(
            &directory,
            &serde_json::json!({"schemaVersion": 1, "payload": oversized})
        )
        .is_err());
        fs::remove_dir_all(directory).expect("temporary directory should be removable");
    }

    #[test]
    fn export_creates_a_pretty_printed_report_without_overwriting() {
        let directory = temporary_directory("export-success");
        let report = serde_json::json!({"schemaVersion": 1, "healthSummary": {"errors": 0}});
        let first = export_report(&directory, &report).expect("first report should export");
        let second = export_report(&directory, &report).expect("second report should export");
        assert_ne!(first.filename, second.filename);
        let contents = fs::read_to_string(directory.join(first.filename))
            .expect("exported report should be readable");
        assert!(contents.contains("\n  \"schemaVersion\": 1"));
        fs::remove_dir_all(directory).expect("temporary directory should be removable");
    }
}
