(() => {
  const tauri = window.__TAURI__;
  if (!tauri?.core?.invoke || !tauri?.event?.listen) {
    throw new Error("Tauri runtime is unavailable");
  }

  const rawInvoke = (command, argumentsObject) => tauri.core.invoke(command, argumentsObject);
  const diagnosticCommands = new Set([
    "record_runtime_failure",
    "record_runtime_warning",
    "record_binding_diagnostic"
  ]);
  const diagnosticFailureCode = (error) => {
    const text = String(error || "").toLowerCase();
    if (text.includes("timed out") || text.includes("timeout")) return "timeout";
    if (text.includes("access") || text.includes("permission") || text.includes("privilege")) return "permission_denied";
    if (text.includes("invalid") || text.includes("unsupported") || text.includes("malformed")) return "invalid_data";
    if (text.includes("unavailable") || text.includes("not available")) return "unavailable";
    if (text.includes("cancelled") || text.includes("canceled")) return "cancelled";
    return "ipc_rejected";
  };
  const invoke = async (command, argumentsObject) => {
    try {
      return await rawInvoke(command, argumentsObject);
    } catch (error) {
      if (!diagnosticCommands.has(command)) {
        void rawInvoke("record_runtime_failure", {
          operation: command,
          code: diagnosticFailureCode(error)
        }).catch(() => {});
      }
      throw error;
    }
  };
  const subscribe = (event, callback) => tauri.event.listen(event, ({ payload }) => callback(payload));

  window.electronAPI = Object.freeze({
    minimize: () => invoke("window_minimize"),
    tray: () => invoke("window_tray"),
    close: async () => {
      // Arm the native watchdog before awaiting renderer-side storage. A
      // wedged WebView or file-system request must not make exit hang forever.
      await invoke("begin_exit");
      if (typeof window.flushPendingDataSaves === "function") {
        await window.flushPendingDataSaves();
      }
      return invoke("window_close");
    },
    sendMacro: (payload) => invoke("execute_macro", { payload }),

    onGlobalKeyDown: (callback) => subscribe("global-keydown", callback),
    onGlobalMouseDown: (callback) => subscribe("global-mousedown", callback),
    onGlobalWheel: (callback) => subscribe("global-wheel", callback),
    onQuitRequested: (callback) => subscribe("quit-requested", callback),
    setGlobalInputFilter: (config, captureAll) => invoke("set_global_input_filter", { config, captureAll }),
    getInputDiagnostics: () => invoke("get_input_diagnostics"),
    recordRuntimeFailure: (operation, code) => rawInvoke("record_runtime_failure", { operation, code }),
    recordRuntimeWarning: (operation, code) => rawInvoke("record_runtime_warning", { operation, code }),
    recordBindingDiagnostic: (stage) => rawInvoke("record_binding_diagnostic", { stage }),
    collectDiagnosticsReport: () => invoke("collect_diagnostics_report"),
    exportDiagnosticsReport: (report) => invoke("export_diagnostics_report", { report }),
    onNativeShortcut: (callback) => subscribe("native-shortcut", callback),
    onNativeMacroStarted: (callback) => subscribe("native-macro-started", callback),
    onNativeMacroFinished: (callback) => subscribe("native-macro-finished", callback),

    toggleOverlay: () => invoke("toggle_overlay"),
    lockOverlay: () => invoke("lock_overlay"),
    unlockOverlay: () => invoke("unlock_overlay"),
    resizeOverlay: (width, height) => invoke("resize_overlay", { width, height }),
    setOverlayPosition: (position) => invoke("set_overlay_position", { position }),
    updateOverlaySettings: (settings) => invoke("update_overlay_settings", { settings }),
    updateOverlay: (data) => invoke("update_overlay", { data }),
    highlightOverlay: (data) => invoke("highlight_overlay", { data }),
    updateSelection: (index) => invoke("update_selection", { index }),
    getOverlaySnapshot: () => invoke("get_overlay_snapshot"),

    loadData: (filename) => invoke("load_data", { filename }),
    saveData: (filename, data) => invoke("save_data", { filename, data }),

    onOverlaySettings: (callback) => subscribe("overlay-settings", callback),
    onSelectionChanged: (callback) => subscribe("selection-changed", callback),
    onHighlightItem: (callback) => subscribe("highlight-item", callback),
    onRenderOverlay: (callback) => subscribe("render-overlay", callback),
    onOverlayLocked: (callback) => subscribe("overlay-locked", callback),
    onOverlayUnlocked: (callback) => subscribe("overlay-unlocked", callback),

    showToast: (payload) => invoke("show_toast", { payload }),
    hideToast: (generation) => invoke("hide_toast", { generation }),
    getLastToast: () => invoke("get_last_toast"),
    onShowToast: (callback) => subscribe("show-toast", callback),

    openSponsor: () => invoke("open_sponsor"),
    closeSponsorWindow: () => invoke("close_sponsor_window"),
    getSponsorUrl: () => invoke("get_sponsor_url"),
    onSponsorUrl: (callback) => subscribe("sponsor-url", callback),

    openOcrHelp: (language) => invoke("open_ocr_help", { language }),
    closeOcrHelpWindow: () => invoke("close_ocr_help_window"),
    getOcrHelpLanguage: () => invoke("get_ocr_help_language"),
    onOcrHelpLang: (callback) => subscribe("ocr-help-lang", callback),

    getAppVersion: () => invoke("get_app_version"),
    checkForUpdates: () => invoke("check_for_updates"),
    openReleaseDownload: () => invoke("open_release_download"),

    getOcrDisplays: () => invoke("get_ocr_displays"),
    startOcrRegionSelect: (displayId) => invoke("start_ocr_region_select", { displayId: displayId == null ? null : Number(displayId) }),
    sendOcrRegionSelected: (region) => invoke("ocr_region_selected", { region }),
    cancelOcrRegionSelect: () => invoke("cancel_ocr_region_select"),
    recognizeOcrRegion: async (region) => {
      try {
        const result = await invoke("recognize_ocr_region", { region });
        return { ok: true, ...result };
      } catch (error) {
        return { ok: false, text: "", confidence: 0, error: String(error) };
      }
    },
    onOcrRegionSelected: (callback) => subscribe("ocr-region-selected", callback)
  });
})();
