use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub const LEGACY_FILES: [&str; 6] = [
    "settings.json",
    "loadout.json",
    "presets.json",
    "custom_strats.json",
    "announcement.json",
    "payurl.json",
];

const MIGRATION_MARKER: &str = ".legacy-import-v1.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub legacy_directory_found: bool,
    pub imported_files: Vec<String>,
    pub skipped_files: Vec<String>,
}

impl MigrationReport {
    fn empty(legacy_directory_found: bool) -> Self {
        Self {
            legacy_directory_found,
            imported_files: Vec::new(),
            skipped_files: Vec::new(),
        }
    }
}

pub fn initialize_data_directory(data_dir: &Path) -> Result<MigrationReport, String> {
    fs::create_dir_all(data_dir)
        .map_err(|error| format!("Cannot create app data directory: {error}"))?;

    let marker_path = data_dir.join(MIGRATION_MARKER);
    if marker_path.exists() {
        return Ok(read_marker(&marker_path).unwrap_or_else(|_| MigrationReport::empty(false)));
    }

    let legacy_directory = legacy_data_directory();
    let mut report = MigrationReport::empty(legacy_directory.is_dir());

    if report.legacy_directory_found {
        for filename in LEGACY_FILES {
            let source = legacy_directory.join(filename);
            let destination = data_dir.join(filename);
            if !source.is_file() || destination.exists() {
                report.skipped_files.push(filename.to_owned());
                continue;
            }

            match fs::copy(&source, &destination) {
                Ok(_) => report.imported_files.push(filename.to_owned()),
                Err(_) => report.skipped_files.push(filename.to_owned()),
            }
        }
    }

    let marker = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("Cannot serialize migration report: {error}"))?;
    fs::write(marker_path, marker)
        .map_err(|error| format!("Cannot write migration marker: {error}"))?;
    Ok(report)
}

pub fn load_json(data_dir: &Path, filename: &str) -> Result<Option<Value>, String> {
    let path = data_file_path(data_dir, filename)?;
    if !path.is_file() {
        return Ok(None);
    }

    let contents =
        fs::read_to_string(path).map_err(|error| format!("Cannot read {filename}: {error}"))?;
    serde_json::from_str(&contents)
        .map(Some)
        .map_err(|error| format!("Cannot parse {filename}: {error}"))
}

pub fn save_json(data_dir: &Path, filename: &str, value: &Value) -> Result<(), String> {
    let path = data_file_path(data_dir, filename)?;
    let serialized = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Cannot serialize {filename}: {error}"))?;
    let temporary_path = path.with_extension("json.pending");

    fs::write(&temporary_path, serialized)
        .map_err(|error| format!("Cannot write {filename}: {error}"))?;
    fs::rename(&temporary_path, &path)
        .map_err(|error| format!("Cannot finalize {filename}: {error}"))
}

fn read_marker(path: &Path) -> Result<MigrationReport, String> {
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&contents).map_err(|error| error.to_string())
}

fn legacy_data_directory() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("HD2-Trigger")
}

fn data_file_path(data_dir: &Path, filename: &str) -> Result<PathBuf, String> {
    if !LEGACY_FILES.contains(&filename) {
        return Err(format!("Unsupported data file: {filename}"));
    }
    Ok(data_dir.join(filename))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn only_known_legacy_files_are_addressable() {
        let root = Path::new("C:/temporary");
        assert!(data_file_path(root, "settings.json").is_ok());
        assert!(data_file_path(root, "../settings.json").is_err());
    }

    #[test]
    fn saves_a_configuration_more_than_once() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!(
            "hd2-macro-terminal-rust-config-{unique}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&data_dir).expect("test directory should be creatable");

        save_json(
            &data_dir,
            "settings.json",
            &serde_json::json!({ "revision": 1 }),
        )
        .expect("first save should work");
        save_json(
            &data_dir,
            "settings.json",
            &serde_json::json!({ "revision": 2 }),
        )
        .expect("replacement save should work");

        assert_eq!(
            load_json(&data_dir, "settings.json").expect("saved JSON should load"),
            Some(serde_json::json!({ "revision": 2 }))
        );
        fs::remove_dir_all(data_dir).expect("test directory should be removable");
    }
}
