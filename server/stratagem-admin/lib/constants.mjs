export const SCHEMA_VERSION = 1;
export const MIN_APP_VERSION = "2.0.6";
export const SIGNING_ALGORITHM = "ed25519";
export const SIGNING_KEY_ID = "catalog-2026-01";

export const GROUPS = Object.freeze([
  "support",
  "orbital",
  "eagle",
  "emplacement",
  "sentry",
  "backpack",
  "vehicle",
  "mission",
]);

export const LIMITS = Object.freeze({
  catalogItems: 512,
  nameLength: 96,
  termsPerField: 32,
  termLength: 96,
  sequenceLength: 32,
  sourceIconBytes: 1024 * 1024,
  normalizedIconBytes: 1024 * 1024,
  catalogBytes: 8 * 1024 * 1024,
  requestBytes: 10 * 1024 * 1024,
  historyEntries: 500,
});

export const ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$/;
export const BUNDLED_ICON_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,159}\.svg$/i;
