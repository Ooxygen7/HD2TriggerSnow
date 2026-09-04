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
const overlaySource = readUiFile("overlay.html");
const sponsorSource = readUiFile("sponsor.html");
const ocrHelpSource = readUiFile("ocr-help.html");
const mainSource = fs.readFileSync(path.join(root, "src-tauri", "src", "main.rs"), "utf8");
const hooksSource = fs.readFileSync(path.join(root, "src-tauri", "src", "hooks.rs"), "utf8");
const inputSource = fs.readFileSync(path.join(root, "src-tauri", "src", "input.rs"), "utf8");
const windowsSource = fs.readFileSync(path.join(root, "src-tauri", "src", "windows.rs"), "utf8");
const runtimeDiagnosticsSource = fs.readFileSync(path.join(root, "src-tauri", "src", "runtime_diagnostics.rs"), "utf8");
const updatesSource = fs.readFileSync(path.join(root, "src-tauri", "src", "updates.rs"), "utf8");
const networkSource = fs.readFileSync(path.join(root, "src-tauri", "src", "network.rs"), "utf8");
const catalogSource = fs.readFileSync(path.join(root, "src-tauri", "src", "catalog.rs"), "utf8");

assert.match(indexSource, /<div id="titlebar" data-tauri-drag-region="deep">/);
assert.match(indexSource, /<div class="titlebar-controls" data-tauri-drag-region="false">/);
assert.match(overlaySource, /<div id="drag-bar" data-tauri-drag-region="deep">/);
assert.match(overlaySource, /<button class="lock-btn" data-tauri-drag-region="false"/);
assert.match(sponsorSource, /<div id="titlebar" data-tauri-drag-region="deep">/);
assert.match(sponsorSource, /<button id="close-btn" data-tauri-drag-region="false"/);
assert.match(ocrHelpSource, /<div id="titlebar" data-tauri-drag-region="deep">/);
assert.match(ocrHelpSource, /<button id="close-btn" data-tauri-drag-region="false"/);

assert.match(
  indexSource,
  /<html lang="zh-CN" data-ui-ready="false" aria-busy="true">/,
  "the unlocalized startup shell must be hidden before the first paint",
);
assert.match(
  indexSource,
  /html\[data-ui-ready="false"\] body > \*\s*{\s*visibility:\s*hidden;/,
  "startup content must remain hidden while settings are loading",
);
assert.match(indexSource, /function finishUiStartup\(\)[\s\S]*dataset\.uiReady = 'true'/);
const initDataStart = indexSource.indexOf("async function initData()");
const initDataEnd = indexSource.indexOf("window.handleToggleOverlay", initDataStart);
assert.notEqual(initDataStart, -1, "could not find main UI initialization");
assert.notEqual(initDataEnd, -1, "could not find main UI initialization end marker");
const initDataSource = indexSource.slice(initDataStart, initDataEnd);
const firstI18nUpdate = initDataSource.indexOf("updateI18n();");
const slowStartupIo = initDataSource.indexOf("await Promise.all([");
assert.ok(
  firstI18nUpdate > -1 && firstI18nUpdate < slowStartupIo,
  "the saved language must be applied before slower startup I/O",
);
assert.match(
  initDataSource,
  /updateI18n\(\);\s*renderMainList\(\{ syncOverlay: true \}\);\s*finishUiStartup\(\);/,
  "the UI may only be revealed after localization and the initial list render",
);
assert.match(
  indexSource,
  /initData\(\)\.catch\([\s\S]*?\)\.finally\(finishUiStartup\);/,
  "startup failures must not leave the window permanently hidden",
);

const tauriConfig = JSON.parse(
  fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const installerHooks = tauriConfig.bundle?.windows?.nsis?.installerHooks;
assert.equal(installerHooks, "windows/installer-hooks.nsh");
assert.equal(tauriConfig.version, "2.0.7", "the bundled version must match the application version");
const packageJson = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const packageLock = JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8"));
assert.equal(packageJson.version, "2.0.7");
assert.equal(packageLock.version, "2.0.7");
assert.equal(packageLock.packages[""].version, "2.0.7");
const installerHookSource = fs.readFileSync(path.join(root, "src-tauri", installerHooks), "utf8");
assert.match(installerHookSource, /CheckIfAppIsRunning "HD2 Macro Terminal\.exe"/);
assert.match(installerHookSource, /CheckIfAppIsRunning "HD2-Trigger\.exe"/);
assert.doesNotMatch(installerHookSource, /CheckIfAppIsRunning "electron\.exe"/i);

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

assert.match(
  indexSource,
  /\.modal-overlay\s*{[\s\S]{0,500}visibility:\s*hidden;[\s\S]{0,500}\.modal-overlay\.active\s*{[^}]*visibility:\s*visible;/,
  "inactive full-window blur layers must be excluded from rendering",
);
assert.match(
  indexSource,
  /#global-status\s*{[\s\S]{0,500}visibility:\s*hidden;[\s\S]{0,500}#global-status\.show\s*{[^}]*visibility:\s*visible;/,
  "the hidden status blur layer must be excluded from rendering",
);
assert.match(
  mainSource,
  /if windows::overlay_is_visible\(&app\)\?\s*{[\s\S]{0,300}persist_current_overlay_position\(&app\)\?;[\s\S]{0,200}destroy_window\(&app, "overlay"\)\?;/,
  "overlay position must be persisted before its idle WebView is destroyed",
);
assert.match(
  windowsSource,
  /pub fn hide_toast[\s\S]{0,500}window\.destroy\(\)/,
  "expired toast WebViews must release their renderer instead of remaining hidden",
);

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
    globalThis.helpers = {
      canonicalGlobalKeyName,
      normalizeHotkeyChord,
      normalizeGlobalInputEvent,
      domBindingInputFromCode,
      bindingChordFromInput,
      escapeHtml,
      safeImageSource,
      normalizeImportedPreset
    };
  `,
  context,
  { filename: "index.html:safety-helpers" },
);

const {
  canonicalGlobalKeyName,
  normalizeHotkeyChord,
  normalizeGlobalInputEvent,
  domBindingInputFromCode,
  bindingChordFromInput,
  escapeHtml,
  safeImageSource,
  normalizeImportedPreset,
} = context.helpers;
assert.equal(canonicalGlobalKeyName("W"), "KeyW");
assert.equal(canonicalGlobalKeyName("Up"), "ArrowUp");
assert.equal(canonicalGlobalKeyName("F24"), "F24");
assert.equal(canonicalGlobalKeyName("NotAKey"), null);
assert.equal(normalizeHotkeyChord("F8"), "F8", "legacy single-key bindings must remain valid");
assert.equal(normalizeHotkeyChord("ControlLeft+1"), "ControlLeft+Digit1");
assert.equal(normalizeHotkeyChord("ControlLeft+ControlLeft"), null);
assert.equal(normalizeHotkeyChord("WheelUp+Digit1"), null, "a wheel edge cannot be held as a modifier");
assert.equal(
  normalizeHotkeyChord("ControlLeft+ShiftLeft+AltLeft+MetaLeft+KeyW+KeyA+KeyS+KeyD+Digit1"),
  null,
  "oversized chords must be rejected",
);
const extraHeldInput = normalizeGlobalInputEvent({
  key: "Digit1",
  pressedInputs: ["ShiftLeft", "KeyW", "ControlLeft", "Digit1"],
});
assert.deepEqual(
  JSON.parse(JSON.stringify(bindingChordFromInput(extraHeldInput))),
  ["ControlLeft", "ShiftLeft", "KeyW", "Digit1"],
  "binding should keep the newest input as the trigger and order held requirements consistently",
);
const domPressedKeys = new Set();
assert.deepEqual(
  JSON.parse(JSON.stringify(domBindingInputFromCode("ControlLeft", domPressedKeys))),
  { key: "ControlLeft", pressedInputs: ["ControlLeft"] },
);
const domChordInput = domBindingInputFromCode("KeyW", domPressedKeys);
assert.deepEqual(
  JSON.parse(JSON.stringify(bindingChordFromInput(domChordInput))),
  ["ControlLeft", "KeyW"],
  "focused-window keyboard capture must preserve held keys for chord binding",
);
assert.equal(domBindingInputFromCode("NotAKey", domPressedKeys), null);
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

const databaseContext = vm.createContext({});
vm.runInContext(
  `${indexSource.slice(databaseStart, databaseEnd)}\nglobalThis.database = defaultStratagemDB;`,
  databaseContext,
  { filename: "index.html:stratagem-database" },
);
const meltagun = JSON.parse(
  vm.runInContext(
    "JSON.stringify(database.find((stratagem) => stratagem.id === 'wpn_meltagun'))",
    databaseContext,
  ),
);
assert.deepEqual(meltagun, {
  id: "wpn_meltagun",
  grp: "support",
  name: { zh: "40-K热熔枪", en: "40-KMeltagun" },
  aliases: ["热熔枪"],
  ocr: ["热熔枪"],
  seq: ["S", "A", "W", "A", "A", "S"],
  icon: "40-K_Meltagun_Stratagem_Icon.svg",
});

const meltagunIconSource = readUiFile("40-K_Meltagun_Stratagem_Icon.svg");
assert.match(meltagunIconSource, /viewBox="0 0 256 256"/);
assert.match(meltagunIconSource, /fill="#011419" fill-opacity="\.75"/);
assert.match(meltagunIconSource, /fill="#53bcda"/);
assert.match(meltagunIconSource, /fill="#ffffee"/);
assert.match(meltagunIconSource, /fill="#5abeda"/);
assert.doesNotMatch(meltagunIconSource, /<image\b/i, "the bundled Meltagun icon must remain vector-only");

const validPreset = normalizeImportedPreset({
  n: "Support",
  l: [{ s: "wpn_mg", h: "F8", l: true }],
});
assert.equal(validPreset.name, "Support");
assert.equal(validPreset.loadout[0].hotkey, "F8");
const chordPreset = normalizeImportedPreset({
  n: "Chord",
  l: [{ s: "wpn_mg", h: "ControlLeft+Digit1", l: false }],
});
assert.equal(chordPreset.loadout[0].hotkey, "ControlLeft+Digit1");
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
assert.match(indexSource, /id="btn-top-diagnostics"[^>]*onclick="openDiagnostics\(\)"/);
assert.match(indexSource, /id="diagnostics-modal"[^>]*role="dialog"[^>]*aria-modal="true"/);
assert.match(indexSource, /id="btn-diagnostics-export"[^>]*onclick="exportDiagnosticsReport\(\)"/);
assert.match(indexSource, /No usernames, full paths, hotkey values, stratagem names, or screenshots/);
assert.match(bridgeSource, /collectDiagnosticsReport:.*collect_diagnostics_report/);
assert.match(bridgeSource, /exportDiagnosticsReport:.*export_diagnostics_report/);
assert.match(bridgeSource, /recordRuntimeFailure:.*record_runtime_failure/);
assert.match(bridgeSource, /recordRuntimeWarning:.*record_runtime_warning/);
assert.match(bridgeSource, /recordBindingDiagnostic:.*record_binding_diagnostic/);
assert.match(mainSource, /async fn\s+collect_diagnostics_report\s*\(/);
assert.match(mainSource, /async fn\s+export_diagnostics_report\s*\(/);
assert.match(mainSource, /fn\s+record_binding_diagnostic\s*\(/);
assert.match(runtimeDiagnosticsSource, /const MAX_RECENT_INCIDENTS:\s*usize\s*=\s*32/);
assert.match(runtimeDiagnosticsSource, /fn\s+allowlisted_operation\s*\(/);
assert.match(runtimeDiagnosticsSource, /fn\s+allowlisted_code\s*\(/);
assert.match(indexSource, /recordBindingDiagnostic\('started'\)/);
assert.match(indexSource, /recordBindingDiagnostic\('input_observed'\)/);
assert.match(indexSource, /recordBindingDiagnostic\('completed'\)/);
assert.match(indexSource, /addEventListener\('error'.*recordRuntimeFailure\('renderer_runtime', 'uncaught_error'\)/);
assert.match(indexSource, /addEventListener\('unhandledrejection'.*recordRuntimeFailure\('renderer_runtime', 'unhandled_rejection'\)/);

const diagnosticI18nStart = indexSource.indexOf("const diagnosticsI18n = {");
const diagnosticI18nEnd = indexSource.indexOf("const defaultStratagemDB", diagnosticI18nStart);
const diagnosticHelperStart = indexSource.indexOf("function fillDiagnosticTemplate");
const diagnosticHelperEnd = indexSource.indexOf("function updateDiagnosticsI18n", diagnosticHelperStart);
assert.notEqual(diagnosticI18nStart, -1, "could not find diagnostics translations");
assert.notEqual(diagnosticI18nEnd, -1, "could not find diagnostics translations end marker");
assert.notEqual(diagnosticHelperStart, -1, "could not find diagnostic health helpers");
assert.notEqual(diagnosticHelperEnd, -1, "could not find diagnostic health helper end marker");
const diagnosticContext = vm.createContext({});
vm.runInContext(
  `
    ${indexSource.slice(diagnosticI18nStart, diagnosticI18nEnd)}
    let currentLang = "en";
    ${indexSource.slice(diagnosticHelperStart, diagnosticHelperEnd)}
    globalThis.diagnosticsHarness = { buildDiagnosticChecks, summarizeDiagnosticChecks };
  `,
  diagnosticContext,
  { filename: "index.html:diagnostic-health" },
);
const healthyReport = {
  application: { version: "2.0.1", architecture: "x86_64", webviewVersion: "140.0" },
  storage: { writable: true, invalidFileCount: 0, recoverableBackupCount: 0, files: [{ status: "valid" }] },
  configuration: { settingsLoaded: true, slotCount: 10, equippedStratagems: 4, boundStratagems: 3, presetCount: 2, duplicateBindingGroups: 0 },
  input: { hookRunning: true, filterInitialized: true, droppedEvents: 0, maxQueueDepth: 2, queueCapacity: 512, processedEvents: 20, shortcutsMatched: 2, unmatchedShortcutEdges: 0, nativeMacrosCompleted: 2, nativeMacrosFailed: 0 },
  runtime: { errorCount: 0, warningCount: 0, recentIncidents: [], binding: { active: false, attemptsStarted: 1, attemptsCompleted: 1, attemptsFailed: 0, cancellationsWithoutInput: 0 } },
  ocr: {
    modelFilesPresent: true,
    selfTest: { ok: true, value: { detectionModelLoaded: true, recognitionModelLoaded: true, dictionaryEntries: 1000 } },
    displays: { ok: true, value: { displayCount: 1, primaryWidth: 1920, primaryHeight: 1080, primaryScaleFactor: 1 } },
  },
  windows: { mainWindowExists: true, overlayWindowExists: false, overlayLocked: false },
  updateService: { reachable: true, validResponse: true, latestVersion: "2.0.1", latencyMs: 50 },
};
const healthyChecks = diagnosticContext.diagnosticsHarness.buildDiagnosticChecks(healthyReport, "en");
assert.equal(healthyChecks.length, 12);
assert.deepEqual(
  JSON.parse(JSON.stringify(diagnosticContext.diagnosticsHarness.summarizeDiagnosticChecks(healthyChecks))),
  { healthy: 12, warnings: 0, errors: 0 },
);
const unhealthyReport = structuredClone(healthyReport);
unhealthyReport.updateService = { reachable: false, validResponse: false, error: "offline" };
unhealthyReport.input.droppedEvents = 2;
unhealthyReport.ocr.modelFilesPresent = false;
unhealthyReport.storage.invalidFileCount = 1;
const unhealthySummary = diagnosticContext.diagnosticsHarness.summarizeDiagnosticChecks(
  diagnosticContext.diagnosticsHarness.buildDiagnosticChecks(unhealthyReport, "zh"),
);
assert.deepEqual(JSON.parse(JSON.stringify(unhealthySummary)), { healthy: 8, warnings: 2, errors: 2 });
const runtimeFailureReport = structuredClone(healthyReport);
runtimeFailureReport.runtime.errorCount = 1;
runtimeFailureReport.runtime.recentIncidents = [{ component: "input", operation: "macro_execution", code: "windows_input_rejected" }];
runtimeFailureReport.runtime.binding.attemptsFailed = 1;
runtimeFailureReport.input.nativeMacrosFailed = 1;
assert.deepEqual(
  JSON.parse(JSON.stringify(diagnosticContext.diagnosticsHarness.summarizeDiagnosticChecks(
    diagnosticContext.diagnosticsHarness.buildDiagnosticChecks(runtimeFailureReport, "zh"),
  ))),
  { healthy: 10, warnings: 0, errors: 2 },
  "runtime and shortcut failures must appear as diagnostic errors",
);

assert.match(bridgeSource, /subscribe\("quit-requested", callback\)/);
assert.match(indexSource, /window\.electronAPI\.onQuitRequested/);
assert.match(bridgeSource, /setGlobalInputFilter:\s*\(config, captureAll\).*set_global_input_filter/);
assert.match(bridgeSource, /getInputDiagnostics:.*get_input_diagnostics/);
assert.match(bridgeSource, /onNativeShortcut:.*native-shortcut/);
assert.match(bridgeSource, /onNativeMacroStarted:.*native-macro-started/);
assert.match(bridgeSource, /onNativeMacroFinished:.*native-macro-finished/);
assert.match(bridgeSource, /checkForUpdates:.*check_for_updates/);
assert.match(bridgeSource, /openReleaseDownload:.*open_release_download/);
assert.match(indexSource, /pendingGlobalInputFilter/);
assert.match(indexSource, /config:\s*buildNativeShortcutConfig\(\)/);
assert.match(indexSource, /payload:\s*buildMacroPayload\(item\)/);
assert.doesNotMatch(indexSource, /function\s+triggerMacro\s*\(/, "JavaScript must not match runtime macro shortcuts");
assert.match(hooksSource, /impl\s+ShortcutState[\s\S]*fn\s+route\s*\(/);
assert.match(hooksSource, /input::prepare_macro\(&binding\.payload\)/);
assert.match(inputSource, /struct\s+PreparedMacro/);
assert.match(inputSource, /dwExtraInfo:\s*SYNTHETIC_INPUT_MARKER/g);
assert.match(hooksSource, /extra_info\s*==\s*input::SYNTHETIC_INPUT_MARKER/);
assert.match(indexSource, /addEventListener\('keydown',\s*handleKeyInput\)/);
assert.match(indexSource, /addEventListener\('keyup',\s*handleKeyUp\)/);
assert.match(
  indexSource,
  /function\s+handleKeyInput\(e\)[\s\S]{0,500}domBindingInputFromCode\(e\.code,[\s\S]{0,300}captureBindingInput\(input\)/,
  "focused keyboard events must reach the binding state machine",
);
assert.match(mainSource, /fn\s+get_input_diagnostics\s*\(/);
assert.match(indexSource, /if \(isOverlayVisible\) \{[\s\S]*gameSettings\.ovExec/);
assert.match(indexSource, /directionOnly:\s*false/, "direction-only mode must be opt-in");
assert.match(indexSource, /directionOnly:\s*!!gameSettings\.directionOnly/);
assert.match(indexSource, /id="direction-only-off"[^>]*onclick="setDirectionOnly\(false\)"/);
assert.match(indexSource, /autoOpenOverlay:\s*false/, "startup overlay must be opt-in");
assert.match(indexSource, /autoLockOverlay:\s*false/, "overlay auto-lock must be opt-in");
assert.match(
  indexSource,
  /id="auto-open-overlay-off"[^>]*onclick="setAutoOpenOverlay\(false\)"[^>]*>关闭</,
  "the startup overlay setting must render off by default",
);
assert.match(
  indexSource,
  /id="auto-lock-overlay-off"[^>]*onclick="setAutoLockOverlay\(false\)"[^>]*>关闭</,
  "the overlay auto-lock setting must render off by default",
);
assert.match(
  indexSource,
  /if \(gameSettings\.autoOpenOverlay === true\) \{\s*await window\.handleToggleOverlay\(\);/,
  "startup must show the overlay only after an explicit opt-in",
);
assert.match(
  indexSource,
  /if \(gameSettings\.autoLockOverlay === true\) \{[\s\S]{0,300}await window\.electronAPI\.lockOverlay\(\);/,
  "every overlay show must honor the explicit auto-lock setting",
);
assert.match(hooksSource, /pressed_inputs:\s*Vec</, "native events must include the held-input snapshot");
assert.match(indexSource, /id="txt-update-title">检测到新版本</);
assert.match(indexSource, /id="btn-update-download"[^>]*>前往下载</);
assert.match(indexSource, /id="btn-update-later"[^>]*>暂不</);
assert.match(indexSource, /void checkForStartupUpdate\(\)/, "startup must trigger a non-blocking update check");
assert.match(indexSource, /messageElement\.textContent = message/, "server text must never be injected as HTML");
assert.match(updatesSource, /update\.unsnow\.online/);
assert.match(updatesSource, /QuickStratagemTool\/releases\/latest/);
assert.match(networkSource, /WinHttpSetTimeouts/);
assert.match(updatesSource, /latest_version <= current_version/);
assert.match(indexSource, /id="txt-catalog-update-title">战备数据库已更新</);
assert.match(indexSource, /void checkForStratagemCatalogUpdate\(\)/);
assert.match(indexSource, /strat\.enabled !== false/);
assert.match(catalogSource, /Catalog signature verification failed/);
assert.match(catalogSource, /BUNDLED_CATALOG_VERSION:\s*u64\s*=\s*1/);

const overlayToggleStart = indexSource.indexOf("window.handleToggleOverlay = async");
const overlayToggleEnd = indexSource.indexOf("window.unlockOverlaySafely", overlayToggleStart);
assert.notEqual(overlayToggleStart, -1, "could not find overlay toggle handler");
assert.notEqual(overlayToggleEnd, -1, "could not find overlay toggle handler end marker");
const overlayCalls = [];
const overlayToasts = [];
const overlayContext = vm.createContext({
  console: { ...console, error() {} },
  window: {
    electronAPI: {
      async toggleOverlay() { overlayCalls.push("toggle"); return true; },
      async resizeOverlay() { overlayCalls.push("resize"); },
      async updateOverlaySettings() { overlayCalls.push("settings"); },
      async updateOverlay() { overlayCalls.push("data"); },
      async updateSelection() { overlayCalls.push("selection"); },
      async lockOverlay() { overlayCalls.push("lock"); },
    },
  },
});
vm.runInContext(
  `
    let isOverlayVisible = false;
    let overlaySelectedIndex = 0;
    const activeLoadout = [];
    const gameSettings = {
      ovWidth: 300,
      ovHeight: 550,
      ovOpacity: 100,
      ovStyle: "text",
      autoLockOverlay: false
    };
    const currentLang = "en";
    const i18n = { en: { msgOverlayFailed: "Overlay operation failed: {error}" } };
    async function syncGlobalInputFilter() { globalThis.overlayCalls.push("filter"); }
    function showToast(message, isError) { globalThis.overlayToasts.push({ message, isError }); }
    ${indexSource.slice(overlayToggleStart, overlayToggleEnd)}
    globalThis.overlayHarness = {
      toggle: window.handleToggleOverlay,
      settings: gameSettings,
      visible: () => isOverlayVisible
    };
  `,
  Object.assign(overlayContext, { overlayCalls, overlayToasts }),
  { filename: "index.html:overlay-toggle" },
);
await overlayContext.overlayHarness.toggle();
assert.deepEqual(overlayCalls, ["toggle", "filter", "resize", "settings", "data", "selection"]);
overlayCalls.length = 0;
overlayContext.overlayHarness.settings.autoLockOverlay = true;
await overlayContext.overlayHarness.toggle();
assert.deepEqual(overlayCalls, ["toggle", "filter", "resize", "settings", "data", "selection", "lock"]);
overlayCalls.length = 0;
overlayContext.window.electronAPI.lockOverlay = async () => { throw new Error("simulated lock failure"); };
await overlayContext.overlayHarness.toggle();
assert.equal(overlayContext.overlayHarness.visible(), true, "a lock failure must not mark a visible overlay as hidden");
assert.equal(overlayToasts.at(-1).isError, true, "an auto-lock failure must be visible to the user");

const updateHelperStart = indexSource.indexOf("function updateUpdateModalText");
const updateHelperEnd = indexSource.indexOf("window.goSponsor", updateHelperStart);
assert.notEqual(updateHelperStart, -1, "could not find update modal helpers");
assert.notEqual(updateHelperEnd, -1, "could not find update modal helper end marker");
const updateElements = new Map();
for (const id of [
  "txt-update-title",
  "txt-update-msg",
  "btn-update-download",
  "btn-update-later",
  "update-modal",
]) {
  const classes = new Set();
  updateElements.set(id, {
    textContent: "",
    focused: false,
    focus() { this.focused = true; },
    classList: {
      add(value) { classes.add(value); },
      remove(value) { classes.delete(value); },
      contains(value) { return classes.has(value); },
    },
  });
}
let updateResponse = null;
let openedReleasePage = 0;
const updateContext = vm.createContext({
  console: { debug() {} },
  document: { getElementById: (id) => updateElements.get(id) },
  showInlineStatus() {},
  window: {
    electronAPI: {
      async checkForUpdates() { return updateResponse; },
      async openReleaseDownload() { openedReleasePage += 1; },
    },
  },
});
vm.runInContext(
  `
    let currentLang = "zh";
    const i18n = { zh: {
      updateTitle: "检测到新版本",
      updateMessage: "当前版本 {current}，最新版本 {latest}。是否前往 GitHub Release 下载？",
      updateDownload: "前往下载",
      updateLater: "暂不",
      msgUpdateOpenFailed: "打开下载页面失败"
    } };
    let pendingUpdateInfo = null;
    ${indexSource.slice(updateHelperStart, updateHelperEnd)}
    globalThis.updateHarness = {
      checkForStartupUpdate,
      closeUpdateModal: window.closeUpdateModal,
      goToUpdateDownload: window.goToUpdateDownload
    };
  `,
  updateContext,
  { filename: "index.html:update-modal" },
);
await updateContext.updateHarness.checkForStartupUpdate();
assert.equal(updateElements.get("update-modal").classList.contains("active"), false);
updateResponse = { currentVersion: "2.0.1", version: "2.0.2" };
await updateContext.updateHarness.checkForStartupUpdate();
assert.equal(updateElements.get("txt-update-title").textContent, "检测到新版本");
assert.match(updateElements.get("txt-update-msg").textContent, /2\.0\.1.*2\.0\.2/);
assert.equal(updateElements.get("update-modal").classList.contains("active"), true);
assert.equal(updateElements.get("btn-update-download").focused, true);
await updateContext.updateHarness.goToUpdateDownload();
assert.equal(openedReleasePage, 1);
assert.equal(updateElements.get("update-modal").classList.contains("active"), false);

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
await bridgeContext.window.electronAPI.checkForUpdates();
await bridgeContext.window.electronAPI.openReleaseDownload();
assert.deepEqual(bridgeCalls.slice(-2), ["check_for_updates", "open_release_download"]);
await bridgeContext.window.electronAPI.collectDiagnosticsReport();
await bridgeContext.window.electronAPI.exportDiagnosticsReport({ schemaVersion: 1 });
assert.deepEqual(bridgeCalls.slice(-2), ["collect_diagnostics_report", "export_diagnostics_report"]);
await bridgeContext.window.electronAPI.recordRuntimeWarning("shortcut_binding", "empty_chord_confirmation");
await bridgeContext.window.electronAPI.recordBindingDiagnostic("started");
assert.deepEqual(bridgeCalls.slice(-2), ["record_runtime_warning", "record_binding_diagnostic"]);

const rejectedBridgeCalls = [];
const rejectedBridgeContext = vm.createContext({
  window: {
    __TAURI__: {
      core: {
        invoke(command, argumentsObject) {
          rejectedBridgeCalls.push({ command, argumentsObject });
          return command === "toggle_overlay"
            ? Promise.reject(new Error("private raw failure text"))
            : Promise.resolve();
        },
      },
      event: { listen: () => Promise.resolve(() => {}) },
    },
  },
});
vm.runInContext(bridgeSource, rejectedBridgeContext, { filename: "bridge.js:failure-tracking" });
await assert.rejects(rejectedBridgeContext.window.electronAPI.toggleOverlay(), /private raw failure text/);
assert.deepEqual(
  JSON.parse(JSON.stringify(rejectedBridgeCalls)),
  [
    { command: "toggle_overlay" },
    { command: "record_runtime_failure", argumentsObject: { operation: "toggle_overlay", code: "ipc_rejected" } },
  ],
  "rejected native operations must record a structured code without copying raw error text",
);

console.log(`UI, native-window, and ${new Set(bundledIcons).size} icon regression tests passed.`);
