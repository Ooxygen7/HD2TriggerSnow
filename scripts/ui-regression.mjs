import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function readUiFile(filename) {
  return fs.readFileSync(path.join(root, "ui", filename), "utf8");
}

function checkInlineScripts(filename) {
  const html = readUiFile(filename);
  const scripts = [...html.matchAll(/<script(?![^>]*\bsrc=)[^>]*>([\s\S]*?)<\/script>/gi)];
  assert.ok(scripts.length > 0, `${filename} should contain an inline script`);
  scripts.forEach((match, index) => {
    new vm.Script(match[1], { filename: `${filename}:inline-${index + 1}` });
  });
}

checkInlineScripts("index.html");
checkInlineScripts("overlay.html");
checkInlineScripts("toast.html");
checkInlineScripts("sponsor.html");
checkInlineScripts("ocr-help.html");
checkInlineScripts("ocr-select.html");

const indexSource = readUiFile("index.html");
const mainSource = fs.readFileSync(path.join(root, "src-tauri", "src", "main.rs"), "utf8");
for (const command of [
  "toggle_overlay",
  "show_toast",
  "open_sponsor",
  "close_sponsor_window",
  "open_ocr_help",
  "close_ocr_help_window",
  "start_ocr_region_select",
  "ocr_region_selected",
  "cancel_ocr_region_select",
]) {
  assert.match(
    mainSource,
    new RegExp(`async\\s+fn\\s+${command}\\s*\\(`),
    `${command} must stay async because it can create a WebView window on Windows`,
  );
}

const helperStart = indexSource.indexOf("const CANONICAL_GLOBAL_KEYS");
const helperEnd = indexSource.indexOf("function normalizeOverlayPosition", helperStart);
assert.notEqual(helperStart, -1, "could not find UI safety helpers");
assert.notEqual(helperEnd, -1, "could not find UI safety helper end marker");

const context = vm.createContext({});
vm.runInContext(
  `
    const MAX_PRESET_NAME_LENGTH = 40;
    const MAX_SLOT_COUNT = 14;
    const EMPTY_IMAGE_SOURCE = "empty-image";
    ${indexSource.slice(helperStart, helperEnd)}
    globalThis.helpers = { canonicalGlobalKeyName, escapeHtml, safeImageSource, normalizeImportedPreset };
  `,
  context,
  { filename: "index.html:safety-helpers" },
);

const { canonicalGlobalKeyName, escapeHtml, safeImageSource, normalizeImportedPreset } = context.helpers;
assert.equal(canonicalGlobalKeyName("W"), "KeyW");
assert.equal(canonicalGlobalKeyName("Up"), "ArrowUp");
assert.equal(canonicalGlobalKeyName("F24"), "F24");
assert.equal(canonicalGlobalKeyName("NotAKey"), null);
assert.equal(escapeHtml('<img src=x onerror="boom">'), "&lt;img src=x onerror=&quot;boom&quot;&gt;");
assert.equal(safeImageSource("Machine_Gun_Stratagem_Icon.svg"), "Machine_Gun_Stratagem_Icon.svg");
assert.equal(safeImageSource("javascript:alert(1)"), "empty-image");
assert.equal(safeImageSource("https://tracker.invalid/icon.svg"), "empty-image");

const databaseStart = indexSource.indexOf("const defaultStratagemDB = [");
const databaseEnd = indexSource.indexOf("let stratagemDB", databaseStart);
assert.notEqual(databaseStart, -1, "could not find the default stratagem database");
assert.notEqual(databaseEnd, -1, "could not find the default stratagem database end marker");
const bundledIcons = [...indexSource.slice(databaseStart, databaseEnd).matchAll(/\bicon:\s*'([^']+)'/g)].map(
  (match) => match[1],
);
assert.ok(bundledIcons.length >= 100, "expected the bundled stratagem icon catalog");
for (const icon of new Set(bundledIcons)) {
  assert.equal(safeImageSource(icon), icon, `bundled icon should be accepted: ${icon}`);
  assert.ok(fs.existsSync(path.join(root, "ui", icon)), `bundled icon is missing: ${icon}`);
}

const validPreset = normalizeImportedPreset({
  n: "Support",
  l: [{ s: "wpn_mg", h: "F8", l: true }],
});
assert.equal(validPreset.name, "Support");
assert.equal(validPreset.loadout[0].hotkey, "F8");
assert.equal(
  normalizeImportedPreset({ n: "Bad", l: [{ s: "wpn_mg", h: '\"><img onerror=boom>', l: false }] }),
  null,
);
assert.equal(
  normalizeImportedPreset({ n: "Too many", l: Array.from({ length: 15 }, () => ({ s: null, h: null })) }),
  null,
);

const saveActorStart = indexSource.indexOf("const pendingDataSaves = new Map();");
const saveActorEnd = indexSource.indexOf("async function initData", saveActorStart);
assert.notEqual(saveActorStart, -1, "could not find the durable save actor");
assert.notEqual(saveActorEnd, -1, "could not find the durable save actor end marker");
const durableWrites = [];
const fallbackWrites = new Map();
const saveContext = vm.createContext({
  console: { ...console, error() {} },
  setTimeout,
  structuredClone,
  localStorage: {
    getItem(key) {
      return fallbackWrites.get(key) ?? null;
    },
    setItem(key, value) {
      fallbackWrites.set(key, value);
    },
    removeItem(key) {
      fallbackWrites.delete(key);
    },
  },
  window: {
    electronAPI: {
      async saveData(filename, data) {
        durableWrites.push({ filename, data: structuredClone(data) });
      },
    },
  },
});
vm.runInContext(
  `
    ${indexSource.slice(saveActorStart, saveActorEnd)}
    globalThis.saveActor = {
      saveDataToLocal,
      loadWithMigration,
      nativeSaveFallbackKey,
      pendingDataSaves
    };
  `,
  saveContext,
  { filename: "index.html:save-actor" },
);
const firstRevision = { revision: 1, nested: { value: "first" } };
saveContext.saveActor.saveDataToLocal("settings.json", firstRevision, "settings-fallback");
firstRevision.nested.value = "mutated-after-queue";
saveContext.saveActor.saveDataToLocal("settings.json", { revision: 2 }, "settings-fallback");
const latestSave = saveContext.saveActor.saveDataToLocal(
  "settings.json",
  { revision: 3 },
  "settings-fallback",
);
await saveContext.window.flushPendingDataSaves();
await latestSave;
assert.deepEqual(
  JSON.parse(JSON.stringify(durableWrites)),
  [
    { filename: "settings.json", data: { revision: 1, nested: { value: "first" } } },
    { filename: "settings.json", data: { revision: 3 } },
  ],
  "the save actor should serialize writes and coalesce only superseded queued revisions",
);
assert.equal(saveContext.saveActor.pendingDataSaves.size, 0, "the save actor should fully drain");
assert.equal(fallbackWrites.size, 0, "successful native saves should not use localStorage fallback");

saveContext.window.electronAPI.saveData = async () => {
  throw new Error("simulated native write failure");
};
await saveContext.saveActor.saveDataToLocal(
  "settings.json",
  { revision: 4 },
  "settings-fallback",
);
await saveContext.window.flushPendingDataSaves();
const nativeFallbackKey = saveContext.saveActor.nativeSaveFallbackKey("settings.json");
assert.deepEqual(JSON.parse(fallbackWrites.get(nativeFallbackKey)), { revision: 4 });

saveContext.window.electronAPI.loadData = async () => ({ revision: 3 });
saveContext.window.electronAPI.saveData = async (filename, data) => {
  durableWrites.push({ filename, data: structuredClone(data) });
};
const recoveredRevision = await saveContext.saveActor.loadWithMigration(
  "settings.json",
  "settings-fallback",
);
assert.equal(recoveredRevision.revision, 4, "a newer failed native save must override the old file");
await saveContext.window.flushPendingDataSaves();
assert.equal(fallbackWrites.has(nativeFallbackKey), false, "recovered native saves should clear fallback");

saveContext.window.electronAPI.saveData = async () => {
  throw new Error("simulated persistent native failure");
};
saveContext.localStorage.setItem = () => {
  throw new Error("simulated localStorage failure");
};
await saveContext.saveActor.saveDataToLocal(
  "presets.json",
  { revision: 5 },
  "presets-fallback",
);
await assert.rejects(
  saveContext.window.flushPendingDataSaves(),
  /Failed to save presets\.json to native storage or local fallback/,
  "shutdown flush must surface a double persistence failure",
);

assert.match(
  indexSource,
  /if \(displayResult\.ok && gameSettings\.ocrDisplayId != null/,
  "display enumeration failures must not erase a saved OCR region",
);
const bridgeSource = readUiFile("bridge.js");
assert.match(bridgeSource, /subscribe\("quit-requested", callback\)/);
assert.match(indexSource, /window\.electronAPI\.onQuitRequested/);
assert.match(bridgeSource, /setGlobalInputFilter:.*set_global_input_filter/);
assert.match(indexSource, /pendingGlobalInputFilter/);
assert.match(indexSource, /if \(isOverlayVisible\) \{[\s\S]*gameSettings\.ovExec/);

const bridgeCalls = [];
let releaseBlockedFlush;
const bridgeContext = vm.createContext({
  window: {
    __TAURI__: {
      core: {
        invoke(command) {
          bridgeCalls.push(command);
          return Promise.resolve();
        },
      },
      event: { listen: () => Promise.resolve(() => {}) },
    },
    flushPendingDataSaves: () => new Promise((resolve) => { releaseBlockedFlush = resolve; }),
  },
});
vm.runInContext(bridgeSource, bridgeContext, { filename: "bridge.js" });
const blockedClose = bridgeContext.window.electronAPI.close();
await new Promise((resolve) => setImmediate(resolve));
assert.deepEqual(bridgeCalls, ["begin_exit"], "native exit watchdog must start before save flushing");
releaseBlockedFlush();
await blockedClose;
assert.deepEqual(bridgeCalls, ["begin_exit", "window_close"]);

console.log(`UI, native-window, and ${new Set(bundledIcons).size} icon regression tests passed.`);
