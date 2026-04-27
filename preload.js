
const { contextBridge, ipcRenderer } = require('electron');

console.log("====== preload.js 已成功加载！======");

contextBridge.exposeInMainWorld('electronAPI', {
    minimize: () => ipcRenderer.send('window-min'),
    tray: () => ipcRenderer.send('window-tray'),
    close: () => ipcRenderer.send('window-close'),
    sendMacro: (payload) => ipcRenderer.send('execute-macro', payload),
    onGlobalKeyDown: (callback) => ipcRenderer.on('global-keydown', (_event, key) => callback(key)),
    onGlobalMouseDown: (callback) => ipcRenderer.on('global-mousedown', (_event, btn) => callback(btn)),
    onGlobalWheel: (callback) => ipcRenderer.on('global-wheel', (_event, dir) => callback(dir)),
    toggleOverlay: () => ipcRenderer.send('toggle-overlay'),
    lockOverlay: () => ipcRenderer.send('lock-overlay'),
    loadData: (filename) => ipcRenderer.invoke('load-data', filename),
    saveData: (filename, data) => ipcRenderer.send('save-data', filename, data),
    unlockOverlay: () => ipcRenderer.send('unlock-overlay'),
    
    resizeOverlay: (w, h) => ipcRenderer.send('resize-overlay', w, h),
    updateOverlaySettings: (settings) => ipcRenderer.send('update-overlay-settings', settings),
    onOverlaySettings: (callback) => ipcRenderer.on('overlay-settings', (_event, settings) => callback(settings)),

    updateOverlay: (data) => ipcRenderer.send('update-overlay', data),
    highlightOverlay: (data) => ipcRenderer.send('highlight-overlay', data),
    updateSelection: (index) => ipcRenderer.send('update-selection', index),
    startOcrRegionSelect: () => ipcRenderer.send('start-ocr-region-select'),
    sendOcrRegionSelected: (region) => ipcRenderer.send('ocr-region-selected', region),
    cancelOcrRegionSelect: () => ipcRenderer.send('cancel-ocr-region-select'),
    recognizeOcrRegion: (region) => ipcRenderer.invoke('recognize-ocr-region', region),
    onOcrRegionSelected: (callback) => ipcRenderer.on('ocr-region-selected', (_event, region) => callback(region)),
    showToast: (payload) => ipcRenderer.send('show-toast', payload),
    onShowToast: (callback) => ipcRenderer.on('show-toast', (_event, payload) => callback(payload)),
    onSelectionChanged: (callback) => ipcRenderer.on('selection-changed', (_event, index) => callback(index)),
    onHighlightItem: (callback) => ipcRenderer.on('highlight-item', (_event, data) => callback(data)),
    onRenderOverlay: (callback) => ipcRenderer.on('render-overlay', (_event, data) => callback(data)),
    onOverlayLocked: (callback) => ipcRenderer.on('overlay-locked', () => callback()),
    onOverlayUnlocked: (callback) => ipcRenderer.on('overlay-unlocked', () => callback()),
    openSponsor: (url) => ipcRenderer.invoke('open-sponsor', url),
    closeSponsorWindow: () => ipcRenderer.send('sponsor-window-close'),
    onSponsorUrl: (callback) => ipcRenderer.on('sponsor-url', (_event, url) => callback(url)),
    openOcrHelp: (lang) => ipcRenderer.invoke('open-ocr-help', lang),
    closeOcrHelpWindow: () => ipcRenderer.send('ocr-help-window-close'),
    onOcrHelpLang: (callback) => ipcRenderer.on('ocr-help-lang', (_event, lang) => callback(lang)),
    getAppVersion: () => ipcRenderer.invoke('get-app-version')
});
