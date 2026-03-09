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
    unlockOverlay: () => ipcRenderer.send('unlock-overlay'),
    updateOverlay: (data) => ipcRenderer.send('update-overlay', data),
    highlightOverlay: (data) => ipcRenderer.send('highlight-overlay', data),
    updateSelection: (index) => ipcRenderer.send('update-selection', index),
    onSelectionChanged: (callback) => ipcRenderer.on('selection-changed', (_event, index) => callback(index)),
    onHighlightItem: (callback) => ipcRenderer.on('highlight-item', (_event, data) => callback(data)),
    onRenderOverlay: (callback) => ipcRenderer.on('render-overlay', (_event, data) => callback(data)),
    onOverlayLocked: (callback) => ipcRenderer.on('overlay-locked', () => callback()),
    onOverlayUnlocked: (callback) => ipcRenderer.on('overlay-unlocked', () => callback())
});