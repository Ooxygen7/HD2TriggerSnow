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
const updatesSource = fs.readFileSync(path.join(root, "src-tauri", "src", "updates.rs"), "utf8");

assert.match(indexSource, /<div id="titlebar" data-tauri-drag-region="deep">/);
assert.match(indexSource, /<div class="titlebar-controls" data-tauri-drag-region="false">/);
assert.match(overlaySource, /<div id="drag-bar" data-tauri-drag-region="deep">/);
assert.match(overlaySource, /<button class="lock-btn" data-tauri-drag-region="false"/);
assert.match(sponsorSource, /<div id="titlebar" data-tauri-drag-region="deep">/);
assert.match(sponsorSource, /<button id="close-btn" data-tauri-drag-region="false"/);
assert.match(ocrHelpSource, /<div id="titlebar" data-tauri-drag-region="deep">/);
assert.match(ocrHelpSource, /<button id="close-btn" data-tauri-drag-region="false"/);

const tauriConfig = JSON.parse(
  fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const installerHooks = tauriConfig.bundle?.windows?.nsis?.installerHooks;
assert.equal(installerHooks, "windows/installer-hooks.nsh");
assert.equal(tauriConfig.version, "2.0.1", "the bundled version must match the GitHub Release version");
const packageJson = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const packageLock = JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8"));
assert.equal(packageJson.version, "2.0.1");
assert.equal(packageLock.version, "2.0.1");
assert.equal(packageLock.packages[""].version, "2.0.1");
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
assert.match(hooksSource, /pressed_inputs:\s*Vec</, "native events must include the held-input snapshot");
assert.match(indexSource, /id="txt-update-title">检测到新版本</);
assert.match(indexSource, /id="btn-update-download"[^>]*>前往下载</);
assert.match(indexSource, /id="btn-update-later"[^>]*>暂不</);
assert.match(indexSource, /void checkForStartupUpdate\(\)/, "startup must trigger a non-blocking update check");
assert.match(indexSource, /messageElement\.textContent = message/, "server text must never be injected as HTML");
assert.match(updatesSource, /update\.unsnow\.online/);
assert.match(updatesSource, /QuickStratagemTool\/releases\/latest/);
assert.match(updatesSource, /WinHttpSetTimeouts/);
assert.match(updatesSource, /latest_version <= current_version/);

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

console.log(`UI, native-window, and ${new Set(bundledIcons).size} icon regression tests passed.`);
