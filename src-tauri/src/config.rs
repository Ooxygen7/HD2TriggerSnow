use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs,
    io::{self, Write},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows::{
    core::PCWSTR,
    Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_WRITE_THROUGH,
    },
};

pub const LEGACY_FILES: [&str; 4] = [
    "settings.json",
    "loadout.json",
    "presets.json",
    "custom_strats.json",
];

const MIGRATION_MARKER: &str = ".legacy-import-v1.json";
const MAX_DATA_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_MARKER_BYTES: u64 = 1024 * 1024;

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

    for filename in LEGACY_FILES {
        recover_data_file(data_dir, filename)?;
    }

    let marker_path = data_dir.join(MIGRATION_MARKER);
    if marker_path.is_file() {
        match read_marker(&marker_path) {
            Ok(report) => return Ok(report),
            Err(_) => quarantine_file(&marker_path),
        }
    }

    let legacy_directory = legacy_data_directory();
    let mut report = MigrationReport::empty(
        legacy_directory
            .as_deref()
            .is_some_and(std::path::Path::is_dir),
    );

    if let Some(legacy_directory) = legacy_directory.filter(|path| path.is_dir()) {
        for filename in LEGACY_FILES {
            let source = legacy_directory.join(filename);
            let destination = data_dir.join(filename);
            if !source.is_file() || destination.exists() {
                report.skipped_files.push(filename.to_owned());
                continue;
            }

            match read_json_path(&source, filename, MAX_DATA_FILE_BYTES)
                .and_then(|value| save_json(data_dir, filename, &value))
            {
                Ok(()) => report.imported_files.push(filename.to_owned()),
                Err(_) => report.skipped_files.push(filename.to_owned()),
            }
        }
    }

    let marker = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("Cannot serialize migration report: {error}"))?;
    atomic_write(&marker_path, &marker, false)
        .map_err(|error| format!("Cannot write migration marker: {error}"))?;
    Ok(report)
}

pub fn load_json(data_dir: &Path, filename: &str) -> Result<Option<Value>, String> {
    let path = data_file_path(data_dir, filename)?;
    if !path.is_file() {
        return Ok(None);
    }

    read_json_path(&path, filename, MAX_DATA_FILE_BYTES).map(Some)
}

pub fn save_json(data_dir: &Path, filename: &str, value: &Value) -> Result<(), String> {
    let path = data_file_path(data_dir, filename)?;
    let serialized = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Cannot serialize {filename}: {error}"))?;
    if serialized.len() as u64 > MAX_DATA_FILE_BYTES {
        return Err(format!(
            "Cannot save {filename}: file exceeds the {} MiB limit",
            MAX_DATA_FILE_BYTES / 1024 / 1024
        ));
    }
    atomic_write(&path, &serialized, true)
        .map_err(|error| format!("Cannot save {filename}: {error}"))
}

fn read_marker(path: &Path) -> Result<MigrationReport, String> {
    read_json_path(path, MIGRATION_MARKER, MAX_MARKER_BYTES)
        .and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
}

fn legacy_data_directory() -> Option<PathBuf> {
    env::var_os("APPDATA")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|path| path.join("HD2-Trigger"))
}

fn read_json_path(path: &Path, label: &str, max_bytes: u64) -> Result<Value, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("Cannot inspect {label}: {error}"))?;
    if metadata.len() > max_bytes {
        return Err(format!("Cannot read {label}: file is too large"));
    }
    let contents = fs::read(path).map_err(|error| format!("Cannot read {label}: {error}"))?;
    serde_json::from_slice(&contents).map_err(|error| format!("Cannot parse {label}: {error}"))
}

fn atomic_write(path: &Path, contents: &[u8], keep_backup: bool) -> io::Result<()> {
    let pending = pending_path(path);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&pending)?;
    file.write_all(contents)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);

    replace_path(
        &pending,
        path,
        keep_backup.then(|| backup_path(path)).as_deref(),
    )
}

fn replace_path(source: &Path, destination: &Path, backup: Option<&Path>) -> io::Result<()> {
    let source_wide = wide_path(source);
    let destination_wide = wide_path(destination);
    if destination.is_file() {
        let backup_wide = backup.map(wide_path);
        // SAFETY: `wide_path` appends a terminating NUL, and all three backing
        // buffers remain alive for the duration of the call. The backup pointer
        // is either null or points into its live buffer. ReplaceFileW does not
        // retain these pointers after returning, and both reserved arguments are
        // passed as null as required by the API.
        unsafe {
            ReplaceFileW(
                PCWSTR(destination_wide.as_ptr()),
                PCWSTR(source_wide.as_ptr()),
                backup_wide
                    .as_ref()
                    .map_or(PCWSTR::null(), |path| PCWSTR(path.as_ptr())),
                REPLACEFILE_WRITE_THROUGH,
                None,
                None,
            )
        }
        .map_err(io::Error::other)
    } else {
        // SAFETY: `wide_path` produces NUL-terminated UTF-16 buffers that stay
        // alive throughout the call. MoveFileExW only reads these paths during
        // the call and does not retain either pointer; the flags are valid for
        // replacing and synchronously flushing the destination entry.
        unsafe {
            MoveFileExW(
                PCWSTR(source_wide.as_ptr()),
                PCWSTR(destination_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(io::Error::other)
    }
}

fn recover_data_file(data_dir: &Path, filename: &str) -> Result<(), String> {
    let path = data_dir.join(filename);
    let pending = pending_path(&path);
    let backup = backup_path(&path);
    if path.is_file() && read_json_path(&path, filename, MAX_DATA_FILE_BYTES).is_ok() {
        let _ = fs::remove_file(pending);
        return Ok(());
    }

    let recovery_source = [&pending, &backup]
        .into_iter()
        .find(|candidate| {
            candidate.is_file() && read_json_path(candidate, filename, MAX_DATA_FILE_BYTES).is_ok()
        })
        .cloned();
    if path.is_file() {
        quarantine_file(&path);
    }
    if let Some(source) = recovery_source {
        let contents =
            fs::read(&source).map_err(|error| format!("Cannot recover {filename}: {error}"))?;
        atomic_write(&path, &contents, false)
            .map_err(|error| format!("Cannot recover {filename}: {error}"))?;
    }
    Ok(())
}

fn quarantine_file(path: &Path) {
    let quarantine = path.with_extension(format!(
        "{}.corrupt",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("data")
    ));
    let _ = fs::remove_file(&quarantine);
    let _ = fs::rename(path, quarantine);
}

fn pending_path(path: &Path) -> PathBuf {
    path.with_extension("json.pending")
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("json.backup")
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
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
        assert_eq!(
            read_json_path(
                &backup_path(&data_dir.join("settings.json")),
                "settings backup",
                MAX_DATA_FILE_BYTES,
            )
            .expect("previous revision should be retained"),
            serde_json::json!({ "revision": 1 })
        );
        fs::remove_dir_all(data_dir).expect("test directory should be removable");
    }

    #[test]
    fn recovers_the_last_good_revision_after_primary_corruption() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!(
            "hd2-macro-terminal-rust-config-recovery-{unique}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&data_dir).expect("test directory should be creatable");
        save_json(
            &data_dir,
            "settings.json",
            &serde_json::json!({ "revision": 1 }),
        )
        .expect("first revision should save");
        save_json(
            &data_dir,
            "settings.json",
            &serde_json::json!({ "revision": 2 }),
        )
        .expect("second revision should save");
        fs::write(data_dir.join("settings.json"), b"{truncated")
            .expect("primary should be corruptible for the test");

        recover_data_file(&data_dir, "settings.json").expect("backup recovery should work");
        assert_eq!(
            load_json(&data_dir, "settings.json").expect("recovered JSON should load"),
            Some(serde_json::json!({ "revision": 1 }))
        );
        assert!(data_dir.join("settings.json.corrupt").is_file());
        fs::remove_dir_all(data_dir).expect("test directory should be removable");
    }

    #[test]
    fn finalizes_a_valid_pending_revision_when_primary_is_missing() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!(
            "hd2-macro-terminal-rust-config-pending-{unique}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&data_dir).expect("test directory should be creatable");
        let primary = data_dir.join("settings.json");
        fs::write(
            pending_path(&primary),
            serde_json::to_vec(&serde_json::json!({ "revision": 3 }))
                .expect("test JSON should serialize"),
        )
        .expect("pending revision should be writable");

        recover_data_file(&data_dir, "settings.json").expect("pending recovery should work");
        assert_eq!(
            load_json(&data_dir, "settings.json").expect("recovered JSON should load"),
            Some(serde_json::json!({ "revision": 3 }))
        );
        assert!(!pending_path(&primary).exists());
        fs::remove_dir_all(data_dir).expect("test directory should be removable");
    }
}
