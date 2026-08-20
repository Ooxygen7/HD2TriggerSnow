import {
  BUNDLED_ICON_PATTERN,
  GROUPS,
  ID_PATTERN,
  LIMITS,
  MIN_APP_VERSION,
  SCHEMA_VERSION,
} from "./constants.mjs";

const NORMALIZED_SVG_MARKER = 'data-hd2-normalized-icon="1"';

function requirePlainObject(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError(`${label} must be an object`);
  }
  return value;
}

function requireString(value, label, maximum, { pattern, allowEmpty = false } = {}) {
  if (typeof value !== "string") throw new TypeError(`${label} must be a string`);
  const normalized = value.trim();
  if (!allowEmpty && normalized.length === 0) throw new TypeError(`${label} is required`);
  if (normalized.length > maximum) throw new TypeError(`${label} is too long`);
  if (pattern && !pattern.test(normalized)) throw new TypeError(`${label} has an invalid format`);
  return normalized;
}

function normalizeTerms(value, label) {
  if (!Array.isArray(value)) throw new TypeError(`${label} must be an array`);
  if (value.length > LIMITS.termsPerField) throw new TypeError(`${label} has too many entries`);
  const seen = new Set();
  const output = [];
  for (const entry of value) {
    const term = requireString(entry, `${label} entry`, LIMITS.termLength);
    const key = term.toLocaleLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      output.push(term);
    }
  }
  return output;
}

function normalizeSequence(value) {
  const joined = Array.isArray(value) ? value.join("") : value;
  const sequence = requireString(joined, "sequence", LIMITS.sequenceLength).toUpperCase();
  if (!/^[WASD]+$/.test(sequence)) throw new TypeError("sequence may contain only W, A, S, and D");
  return sequence.split("");
}

function validateNormalizedSvg(base64) {
  if (typeof base64 !== "string" || !/^[A-Za-z0-9+/]+={0,2}$/.test(base64)) {
    throw new TypeError("icon base64 is invalid");
  }
  const bytes = Buffer.from(base64, "base64");
  if (bytes.length === 0 || bytes.length > LIMITS.normalizedIconBytes) {
    throw new TypeError("normalized icon has an invalid size");
  }
  const svg = bytes.toString("utf8");
  if (!svg.includes(NORMALIZED_SVG_MARKER) || !svg.startsWith("<svg ")) {
    throw new TypeError("icon was not normalized by the catalog service");
  }
  if (/<(?:script|foreignObject|iframe|object|embed|audio|video|style)\b/i.test(svg)) {
    throw new TypeError("normalized icon contains forbidden content");
  }
  if (/\bon[a-z]+\s*=|(?:href|src)\s*=\s*["'](?!data:image\/png;base64,)/i.test(svg)) {
    throw new TypeError("normalized icon contains an unsafe reference");
  }
  return base64;
}

export function iconSource(icon) {
  if (icon.kind === "bundled") return icon.value;
  return `data:${icon.mediaType};base64,${icon.base64}`;
}

export function normalizeIcon(value) {
  const icon = requirePlainObject(value, "icon");
  if (icon.kind === "bundled") {
    return {
      kind: "bundled",
      value: requireString(icon.value, "bundled icon", 164, { pattern: BUNDLED_ICON_PATTERN }),
    };
  }
  if (icon.kind === "data") {
    if (icon.mediaType !== "image/svg+xml") throw new TypeError("remote icons must be normalized SVG");
    return {
      kind: "data",
      mediaType: "image/svg+xml",
      base64: validateNormalizedSvg(icon.base64),
    };
  }
  throw new TypeError("icon kind is unsupported");
}

export function normalizeItem(value, fallbackOrder = 0) {
  const item = requirePlainObject(value, "stratagem");
  const id = requireString(item.id, "id", 100, { pattern: ID_PATTERN });
  if (id.toLowerCase().startsWith("custom_")) throw new TypeError("remote IDs cannot use the custom_ prefix");
  const grp = requireString(item.grp, "group", 32);
  if (!GROUPS.includes(grp)) throw new TypeError("group is unsupported");
  const name = requirePlainObject(item.name, "name");
  const orderValue = item.order == null ? fallbackOrder : Number(item.order);
  if (!Number.isSafeInteger(orderValue) || orderValue < 0 || orderValue > 100000) {
    throw new TypeError("order must be an integer between 0 and 100000");
  }
  return {
    id,
    grp,
    name: {
      zh: requireString(name.zh, "Chinese name", LIMITS.nameLength),
      en: requireString(name.en, "English name", LIMITS.nameLength),
    },
    aliases: normalizeTerms(item.aliases ?? [], "aliases"),
    ocr: normalizeTerms(item.ocr ?? [], "OCR terms"),
    seq: normalizeSequence(item.seq),
    icon: normalizeIcon(item.icon),
    enabled: item.enabled !== false,
    order: orderValue,
  };
}

export function normalizeItems(value) {
  if (!Array.isArray(value)) throw new TypeError("items must be an array");
  if (value.length === 0 || value.length > LIMITS.catalogItems) {
    throw new TypeError(`items must contain between 1 and ${LIMITS.catalogItems} entries`);
  }
  const ids = new Set();
  const items = value.map((item, index) => {
    const normalized = normalizeItem(item, index);
    const key = normalized.id.toLocaleLowerCase();
    if (ids.has(key)) throw new TypeError(`duplicate stratagem ID: ${normalized.id}`);
    ids.add(key);
    return normalized;
  });
  return items.sort((left, right) => left.order - right.order || left.id.localeCompare(right.id));
}

export function createCatalog({ version, publishedAt, items }) {
  if (!Number.isSafeInteger(version) || version < 1) throw new TypeError("catalog version is invalid");
  const timestamp = new Date(publishedAt);
  if (!Number.isFinite(timestamp.valueOf())) throw new TypeError("publishedAt is invalid");
  return {
    schemaVersion: SCHEMA_VERSION,
    catalogVersion: version,
    publishedAt: timestamp.toISOString(),
    minAppVersion: MIN_APP_VERSION,
    items: normalizeItems(items),
  };
}

export function serializeCatalog(catalog) {
  const bytes = Buffer.from(`${JSON.stringify(catalog, null, 2)}\n`, "utf8");
  if (bytes.length > LIMITS.catalogBytes) throw new TypeError("catalog exceeds the size limit");
  return bytes;
}

export function clientItem(item) {
  return {
    id: item.id,
    grp: item.grp,
    name: item.name,
    aliases: item.aliases,
    ocr: item.ocr,
    seq: item.seq,
    icon: iconSource(item.icon),
    enabled: item.enabled,
  };
}
