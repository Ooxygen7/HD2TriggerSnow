use base64::{engine::general_purpose, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, path::Path};

use crate::{config, network};

const ENDPOINT_HOST: &str = "update.unsnow.online";
const MANIFEST_PATH: &str = "/api/v1/stratagems/manifest";
const CACHE_FILENAME: &str = "stratagem-catalog-cache.json";
const SCHEMA_VERSION: u32 = 1;
const BUNDLED_CATALOG_VERSION: u64 = 1;
const SIGNING_ALGORITHM: &str = "ed25519";
const SIGNING_KEY_ID: &str = "catalog-2026-01";
const PUBLIC_KEY_BASE64URL: &str = "mF-DaQGEfIFDW4oHM7M1Cq8YqHJPzkXdq_3bkPV3jdA";
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 12 * 1024 * 1024;
const MAX_ITEMS: usize = 512;
const MAX_ICON_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    catalog_version: u64,
    published_at: String,
    min_app_version: String,
    item_count: usize,
    sha256: String,
    signature: String,
    signing_algorithm: String,
    key_id: String,
    catalog_path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    schema_version: u32,
    catalog_version: u64,
    published_at: String,
    min_app_version: String,
    items: Vec<CatalogItem>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogItem {
    id: String,
    grp: String,
    name: LocalizedName,
    aliases: Vec<String>,
    ocr: Vec<String>,
    seq: Vec<String>,
    icon: CatalogIcon,
    enabled: bool,
    order: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LocalizedName {
    zh: String,
    en: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum CatalogIcon {
    Bundled {
        value: String,
    },
    Data {
        #[serde(rename = "mediaType")]
        media_type: String,
        base64: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheEnvelope {
    manifest: Manifest,
    catalog_base64: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPayload {
    pub catalog_version: u64,
    pub published_at: String,
    pub items: Vec<ClientItem>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientItem {
    id: String,
    grp: String,
    name: LocalizedName,
    aliases: Vec<String>,
    ocr: Vec<String>,
    seq: Vec<String>,
    icon: String,
    enabled: bool,
    order: u32,
}

pub fn load_cached(data_dir: &Path) -> Result<Option<CatalogPayload>, String> {
    let primary = data_dir.join(CACHE_FILENAME);
    let backup = primary.with_extension("json.backup");
    if primary.is_file() {
        match load_envelope(&primary) {
            Ok(payload) => return Ok(Some(payload)),
            Err(primary_error) if backup.is_file() => {
                return load_envelope(&backup).map(Some).map_err(|backup_error| {
                    format!("Catalog cache and backup are invalid: {primary_error}; {backup_error}")
                });
            }
            Err(error) => return Err(format!("Catalog cache is invalid: {error}")),
        }
    }
    if backup.is_file() {
        return load_envelope(&backup).map(Some);
    }
    Ok(None)
}

pub fn check_for_update(data_dir: &Path) -> Result<Option<CatalogPayload>, String> {
    let current_version = load_cached(data_dir)
        .ok()
        .flatten()
        .map_or(BUNDLED_CATALOG_VERSION, |catalog| {
            catalog.catalog_version.max(BUNDLED_CATALOG_VERSION)
        });
    let manifest_bytes = network::fetch_https(
        ENDPOINT_HOST,
        MANIFEST_PATH,
        "application/json",
        MAX_MANIFEST_BYTES,
    )?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("Catalog manifest is invalid JSON: {error}"))?;
    validate_manifest(&manifest)?;
    if manifest.catalog_version <= current_version {
        return Ok(None);
    }
    let catalog_bytes = network::fetch_https(
        ENDPOINT_HOST,
        &manifest.catalog_path,
        "application/json",
        MAX_CATALOG_BYTES,
    )?;
    let payload = verify_bundle(&manifest, &catalog_bytes, &production_key()?)?;
    save_envelope(data_dir, manifest, &catalog_bytes)?;
    Ok(Some(payload))
}

fn production_key() -> Result<VerifyingKey, String> {
    let bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(PUBLIC_KEY_BASE64URL)
        .map_err(|error| format!("Embedded catalog key is invalid: {error}"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "Embedded catalog key has an invalid length".to_owned())?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|error| format!("Embedded catalog key is invalid: {error}"))
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema_version != SCHEMA_VERSION {
        return Err("Catalog manifest schema is unsupported".to_owned());
    }
    if manifest.catalog_version < 1 || manifest.item_count == 0 || manifest.item_count > MAX_ITEMS {
        return Err("Catalog manifest counts are invalid".to_owned());
    }
    if manifest.signing_algorithm != SIGNING_ALGORITHM || manifest.key_id != SIGNING_KEY_ID {
        return Err("Catalog manifest signing key is unsupported".to_owned());
    }
    if manifest.catalog_path != format!("/api/v1/stratagems/catalog/{}", manifest.catalog_version) {
        return Err("Catalog manifest path is invalid".to_owned());
    }
    if manifest.published_at.len() < 20 || manifest.published_at.len() > 40 {
        return Err("Catalog publication time is invalid".to_owned());
    }
    if manifest.sha256.len() != 64 || !manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Catalog digest is invalid".to_owned());
    }
    let minimum = Version::parse(&manifest.min_app_version)
        .map_err(|error| format!("Catalog minimum app version is invalid: {error}"))?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("Application version is invalid: {error}"))?;
    if minimum > current {
        return Err(format!("Catalog requires app version {minimum}"));
    }
    Ok(())
}

fn verify_bundle(
    manifest: &Manifest,
    catalog_bytes: &[u8],
    key: &VerifyingKey,
) -> Result<CatalogPayload, String> {
    validate_manifest(manifest)?;
    if catalog_bytes.is_empty() || catalog_bytes.len() > MAX_CATALOG_BYTES {
        return Err("Catalog size is invalid".to_owned());
    }
    let digest = Sha256::digest(catalog_bytes);
    let digest_hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if !digest_hex.eq_ignore_ascii_case(&manifest.sha256) {
        return Err("Catalog digest does not match the manifest".to_owned());
    }
    let signature = general_purpose::STANDARD
        .decode(&manifest.signature)
        .map_err(|error| format!("Catalog signature encoding is invalid: {error}"))?;
    let signature = Signature::from_slice(&signature)
        .map_err(|error| format!("Catalog signature length is invalid: {error}"))?;
    key.verify(catalog_bytes, &signature)
        .map_err(|_| "Catalog signature verification failed".to_owned())?;
    let catalog: Catalog = serde_json::from_slice(catalog_bytes)
        .map_err(|error| format!("Catalog is invalid JSON: {error}"))?;
    validate_catalog(manifest, catalog)
}

fn validate_catalog(manifest: &Manifest, catalog: Catalog) -> Result<CatalogPayload, String> {
    if catalog.schema_version != SCHEMA_VERSION
        || catalog.catalog_version != manifest.catalog_version
        || catalog.published_at != manifest.published_at
        || catalog.min_app_version != manifest.min_app_version
        || catalog.items.len() != manifest.item_count
    {
        return Err("Catalog metadata does not match the manifest".to_owned());
    }
    if catalog.items.is_empty() || catalog.items.len() > MAX_ITEMS {
        return Err("Catalog item count is invalid".to_owned());
    }
    let mut ids = HashSet::with_capacity(catalog.items.len());
    let mut output = Vec::with_capacity(catalog.items.len());
    for item in catalog.items {
        validate_id(&item.id)?;
        if !ids.insert(item.id.to_ascii_lowercase()) {
            return Err(format!("Catalog contains duplicate ID {}", item.id));
        }
        if !matches!(
            item.grp.as_str(),
            "support"
                | "orbital"
                | "eagle"
                | "emplacement"
                | "sentry"
                | "backpack"
                | "vehicle"
                | "mission"
        ) {
            return Err(format!("Catalog group is invalid for {}", item.id));
        }
        validate_text(&item.name.zh, 96, "Chinese name")?;
        validate_text(&item.name.en, 96, "English name")?;
        validate_terms(&item.aliases, "aliases")?;
        validate_terms(&item.ocr, "OCR terms")?;
        if item.seq.is_empty()
            || item.seq.len() > 32
            || !item
                .seq
                .iter()
                .all(|key| matches!(key.as_str(), "W" | "A" | "S" | "D"))
        {
            return Err(format!("Catalog sequence is invalid for {}", item.id));
        }
        if item.order > 100_000 {
            return Err(format!("Catalog order is invalid for {}", item.id));
        }
        let icon = validate_icon(item.icon)?;
        output.push(ClientItem {
            id: item.id,
            grp: item.grp,
            name: item.name,
            aliases: item.aliases,
            ocr: item.ocr,
            seq: item.seq,
            icon,
            enabled: item.enabled,
            order: item.order,
        });
    }
    output.sort_by(|left, right| {
        left.order
            .cmp(&right.order)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(CatalogPayload {
        catalog_version: catalog.catalog_version,
        published_at: catalog.published_at,
        items: output,
    })
}

fn validate_id(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= 100
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        && !value.to_ascii_lowercase().starts_with("custom_");
    if valid {
        Ok(())
    } else {
        Err("Catalog contains an invalid ID".to_owned())
    }
}

fn validate_text(value: &str, maximum: usize, label: &str) -> Result<(), String> {
    if value.trim() != value || value.is_empty() || value.chars().count() > maximum {
        return Err(format!("Catalog {label} is invalid"));
    }
    Ok(())
}

fn validate_terms(terms: &[String], label: &str) -> Result<(), String> {
    if terms.len() > 32 {
        return Err(format!("Catalog {label} contains too many values"));
    }
    let mut seen = HashSet::with_capacity(terms.len());
    for term in terms {
        validate_text(term, 96, label)?;
        if !seen.insert(term.to_lowercase()) {
            return Err(format!("Catalog {label} contains duplicates"));
        }
    }
    Ok(())
}

fn validate_icon(icon: CatalogIcon) -> Result<String, String> {
    match icon {
        CatalogIcon::Bundled { value } => {
            let valid = value.len() <= 164
                && value.to_ascii_lowercase().ends_with(".svg")
                && value.as_bytes()[0].is_ascii_alphanumeric()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'));
            if !valid {
                return Err("Catalog bundled icon path is invalid".to_owned());
            }
            Ok(value)
        }
        CatalogIcon::Data { media_type, base64 } => {
            if media_type != "image/svg+xml" {
                return Err("Catalog remote icon type is invalid".to_owned());
            }
            let bytes = general_purpose::STANDARD
                .decode(&base64)
                .map_err(|error| format!("Catalog icon encoding is invalid: {error}"))?;
            if bytes.is_empty() || bytes.len() > MAX_ICON_BYTES {
                return Err("Catalog icon size is invalid".to_owned());
            }
            let svg =
                std::str::from_utf8(&bytes).map_err(|_| "Catalog icon is not UTF-8".to_owned())?;
            let lower = svg.to_ascii_lowercase();
            if !svg.starts_with("<svg ")
                || !svg.ends_with("</svg>")
                || !svg.contains("data-hd2-normalized-icon=\"1\"")
                || !svg.contains("data:image/png;base64,")
                || [
                    "<script",
                    "<foreignobject",
                    "<iframe",
                    "<object",
                    "<embed",
                    " on",
                ]
                .iter()
                .any(|needle| lower.contains(needle))
            {
                return Err("Catalog icon wrapper is invalid".to_owned());
            }
            Ok(format!("data:image/svg+xml;base64,{base64}"))
        }
    }
}

fn load_envelope(path: &Path) -> Result<CatalogPayload, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("Cannot inspect catalog cache: {error}"))?;
    if metadata.len() == 0 || metadata.len() > MAX_CACHE_BYTES {
        return Err("Catalog cache size is invalid".to_owned());
    }
    let bytes = fs::read(path).map_err(|error| format!("Cannot read catalog cache: {error}"))?;
    let envelope: CacheEnvelope = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Cannot parse catalog cache: {error}"))?;
    let catalog_bytes = general_purpose::STANDARD
        .decode(envelope.catalog_base64)
        .map_err(|error| format!("Cached catalog encoding is invalid: {error}"))?;
    verify_bundle(&envelope.manifest, &catalog_bytes, &production_key()?)
}

fn save_envelope(data_dir: &Path, manifest: Manifest, catalog_bytes: &[u8]) -> Result<(), String> {
    fs::create_dir_all(data_dir)
        .map_err(|error| format!("Cannot create catalog cache directory: {error}"))?;
    let path = data_dir.join(CACHE_FILENAME);
    let keep_backup = path.is_file() && load_envelope(&path).is_ok();
    if path.is_file() && !keep_backup {
        let quarantine = path.with_extension("json.corrupt");
        let _ = fs::remove_file(&quarantine);
        fs::rename(&path, &quarantine)
            .map_err(|error| format!("Cannot quarantine invalid catalog cache: {error}"))?;
    }
    let envelope = CacheEnvelope {
        manifest,
        catalog_base64: general_purpose::STANDARD.encode(catalog_bytes),
    };
    let bytes = serde_json::to_vec(&envelope)
        .map_err(|error| format!("Cannot serialize catalog cache: {error}"))?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return Err("Catalog cache exceeds the size limit".to_owned());
    }
    config::atomic_write(&path, &bytes, keep_backup)
        .map_err(|error| format!("Cannot save catalog cache: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn signed_fixture() -> (Manifest, Vec<u8>, VerifyingKey) {
        let signing = SigningKey::from_bytes(&[7_u8; 32]);
        let published_at = "2026-08-20T00:00:00.000Z";
        let catalog = serde_json::json!({
            "schemaVersion": 1,
            "catalogVersion": 2,
            "publishedAt": published_at,
            "minAppVersion": env!("CARGO_PKG_VERSION"),
            "items": [{
                "id": "wpn_test",
                "grp": "support",
                "name": { "zh": "测试", "en": "Test" },
                "aliases": ["别名"],
                "ocr": ["测试"],
                "seq": ["W", "A"],
                "icon": { "kind": "bundled", "value": "Test.svg" },
                "enabled": true,
                "order": 0
            }]
        });
        let bytes = serde_json::to_vec(&catalog).unwrap();
        let digest = Sha256::digest(&bytes);
        let manifest = Manifest {
            schema_version: 1,
            catalog_version: 2,
            published_at: published_at.to_owned(),
            min_app_version: env!("CARGO_PKG_VERSION").to_owned(),
            item_count: 1,
            sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
            signature: general_purpose::STANDARD.encode(signing.sign(&bytes).to_bytes()),
            signing_algorithm: SIGNING_ALGORITHM.to_owned(),
            key_id: SIGNING_KEY_ID.to_owned(),
            catalog_path: "/api/v1/stratagems/catalog/2".to_owned(),
        };
        (manifest, bytes, signing.verifying_key())
    }

    #[test]
    fn verifies_a_valid_signed_catalog() {
        let (manifest, bytes, key) = signed_fixture();
        let payload = verify_bundle(&manifest, &bytes, &key).unwrap();
        assert_eq!(payload.catalog_version, 2);
        assert_eq!(payload.items.len(), 1);
    }

    #[test]
    fn rejects_tampering_and_manifest_mismatch() {
        let (mut manifest, mut bytes, key) = signed_fixture();
        bytes.push(b' ');
        assert!(verify_bundle(&manifest, &bytes, &key).is_err());
        let (_, bytes, key) = signed_fixture();
        manifest.item_count = 2;
        assert!(verify_bundle(&manifest, &bytes, &key).is_err());
    }

    #[test]
    fn rejects_unsafe_icon_paths_and_reserved_ids() {
        assert!(validate_id("custom_remote").is_err());
        assert!(validate_icon(CatalogIcon::Bundled {
            value: "../bad.svg".to_owned()
        })
        .is_err());
    }

    #[test]
    #[ignore = "requires the public update.unsnow.online endpoint"]
    fn public_catalog_is_signed_by_the_embedded_production_key() {
        let manifest_bytes = network::fetch_https(
            ENDPOINT_HOST,
            MANIFEST_PATH,
            "application/json",
            MAX_MANIFEST_BYTES,
        )
        .expect("the public manifest should respond");
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
            .expect("the public manifest should be valid JSON");
        let catalog_bytes = network::fetch_https(
            ENDPOINT_HOST,
            &manifest.catalog_path,
            "application/json",
            MAX_CATALOG_BYTES,
        )
        .expect("the public catalog should respond");
        let catalog = verify_bundle(&manifest, &catalog_bytes, &production_key().unwrap())
            .expect("the public catalog signature should verify");
        assert_eq!(catalog.items.len(), 101);
    }
}
