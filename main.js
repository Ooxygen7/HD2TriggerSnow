const { app, BrowserWindow, ipcMain, Tray, Menu, nativeImage } = require('electron');
const path = require('path');
const { uIOhook } = require('uiohook-napi'); 
const { keyboard, Key } = require('@nut-tree/nut-js');

let mainWindow;
let overlayWindow; 
let tray = null;
let isQuitting = false; 

keyboard.config.autoDelayMs = 40; 

const uioToChar = {
    30: "A", 48: "B", 46: "C", 32: "D", 18: "E", 33: "F", 34: "G", 35: "H",
    23: "I", 36: "J", 37: "K", 38: "L", 50: "M", 49: "N", 24: "O", 25: "P",
    16: "Q", 19: "R", 31: "S", 20: "T", 22: "U", 47: "V", 17: "W", 45: "X",
    21: "Y", 44: "Z",
    2: "Digit1", 3: "Digit2", 4: "Digit3", 5: "Digit4", 6: "Digit5", 
    7: "Digit6", 8: "Digit7", 9: "Digit8", 10: "Digit9", 11: "Digit0",
    82: "Numpad0", 79: "Numpad1", 80: "Numpad2", 81: "Numpad3", 75: "Numpad4",
    76: "Numpad5", 77: "Numpad6", 71: "Numpad7", 72: "Numpad8", 73: "Numpad9",
    59: "F1", 60: "F2", 61: "F3", 62: "F4", 63: "F5", 64: "F6",
    65: "F7", 66: "F8", 67: "F9", 68: "F10", 87: "F11", 88: "F12",
    57: "Space", 15: "Tab", 58: "CapsLock",
    42: "ShiftLeft", 29: "ControlLeft", 56: "AltLeft",
    54: "ShiftRight", 97: "ControlRight", 100: "AltRight"
};

const nutKeyMap = {
    'W': Key.W, 'S': Key.S, 'A': Key.A, 'D': Key.D,
    'Up': Key.Up, 'Down': Key.Down, 'Left': Key.Left, 'Right': Key.Right,
    'ArrowUp': Key.Up, 'ArrowDown': Key.Down, 'ArrowLeft': Key.Left, 'ArrowRight': Key.Right,
    'ControlLeft': Key.LeftControl, 'ControlRight': Key.RightControl,
    'AltLeft': Key.LeftAlt, 'AltRight': Key.RightAlt,
    'ShiftLeft': Key.LeftShift, 'ShiftRight': Key.RightShift,
    'CapsLock': Key.CapsLock, 'Tab': Key.Tab, 'Space': Key.Space
};

function createWindow() {
    mainWindow = new BrowserWindow({
        width: 670, height: 838, minWidth: 667, minHeight: 838,
        frame: false, backgroundColor: '#000000',
        webPreferences: { preload: path.join(__dirname, 'preload.js'), contextIsolation: true }
    });
    mainWindow.loadFile('index.html');

    mainWindow.on('close', (event) => {
        if (!isQuitting) {
            event.preventDefault();
            mainWindow.hide();
        }
    });
}

function createOverlayWindow() {
    overlayWindow = new BrowserWindow({
        width: 300, height: 550, x: 50, y: 50, 
        transparent: true, frame: false, alwaysOnTop: true, skipTaskbar: true, resizable: false, show: false,       
        webPreferences: { preload: path.join(__dirname, 'preload.js'), contextIsolation: true }
    });
    overlayWindow.setAlwaysOnTop(true, 'screen-saver'); 
    overlayWindow.loadFile('overlay.html');
    overlayWindow.on('close', (event) => {
        if (!isQuitting) {
            event.preventDefault();
            overlayWindow.hide();
        }
    });
}

app.whenReady().then(() => {
    createWindow();
    createOverlayWindow();
    const showMainWindow = () => {
        if (mainWindow && !mainWindow.isDestroyed()) {
            mainWindow.show();
            if (mainWindow.isMinimized()) mainWindow.restore();
        }
    };

    try {
        const iconPath = path.join(__dirname, 'icon.png');
        const appIcon = nativeImage.createFromPath(iconPath);
        if (!appIcon.isEmpty()) {
            tray = new Tray(appIcon); 
            tray.setContextMenu(Menu.buildFromTemplate([
                { label: '显示界面', click: showMainWindow },
                { label: '完全退出', click: () => { 
                    isQuitting = true; 
                    uIOhook.stop(); 
                    app.quit(); 
                } }
            ]));
            tray.on('click', showMainWindow);
        }
    } catch (e) {}

    ipcMain.on('window-min', () => mainWindow.minimize());
    ipcMain.on('window-tray', () => mainWindow.hide());
    
    ipcMain.on('window-close', () => { 
        isQuitting = true; 
        uIOhook.stop(); 
        app.quit(); 
    });

    ipcMain.on('toggle-overlay', () => {
        if (overlayWindow && !overlayWindow.isDestroyed()) {
            overlayWindow.isVisible() ? overlayWindow.hide() : overlayWindow.showInactive();
        }
    });

    ipcMain.on('lock-overlay', () => { if (overlayWindow && !overlayWindow.isDestroyed()) { overlayWindow.setIgnoreMouseEvents(true, { forward: true }); overlayWindow.webContents.send('overlay-locked'); } });
    ipcMain.on('unlock-overlay', () => { if (overlayWindow && !overlayWindow.isDestroyed()) { overlayWindow.setIgnoreMouseEvents(false); overlayWindow.showInactive(); overlayWindow.webContents.send('overlay-unlocked'); } });
    
    ipcMain.on('update-overlay', (event, data) => { if (overlayWindow && !overlayWindow.isDestroyed()) overlayWindow.webContents.send('render-overlay', data); });
    ipcMain.on('highlight-overlay', (event, data) => { if (overlayWindow && !overlayWindow.isDestroyed()) overlayWindow.webContents.send('highlight-item', data); });
    ipcMain.on('update-selection', (event, index) => { if (overlayWindow && !overlayWindow.isDestroyed()) overlayWindow.webContents.send('selection-changed', index); });

    uIOhook.on('keydown', (e) => {
        const char = uioToChar[e.keycode]; 
        if (char && mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send('global-keydown', char);
    });

    uIOhook.on('mousedown', (e) => {
        let mBtn = "";
        if (e.button === 3) mBtn = "MouseMiddle"; 
        else if (e.button === 4) mBtn = "MouseSide1"; 
        else if (e.button === 5) mBtn = "MouseSide2"; 
        else if (e.button === 6) mBtn = "MouseSide3"; 
        if (mBtn && mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send('global-mousedown', mBtn);
    });

    uIOhook.on('wheel', (e) => {
        const dir = e.rotation > 0 ? 1 : -1;
        if (mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send('global-wheel', dir);
    });

    uIOhook.start();
});

const delay = (ms) => new Promise(resolve => setTimeout(resolve, ms));

ipcMain.on('execute-macro', async (event, payload) => {
    const { menuKey, menuMode, sequence, delayMs } = payload;
    const mKey = nutKeyMap[menuKey] || Key[menuKey] || Key.LeftControl;
    const baseDelay = Math.max(10, parseInt(delayMs) || 10);
    const PRESS_DELAY = Math.max(5, Math.floor(baseDelay / 2)); 
    const INTERVAL_DELAY = Math.max(5, Math.floor(baseDelay / 2));
    const MENU_OPEN_DELAY = Math.max(50, baseDelay * 5); 

    try {
        if (menuMode === 'hold') {
            await keyboard.pressKey(mKey); await delay(MENU_OPEN_DELAY); 
            for (let k of sequence) { let pressKey = nutKeyMap[k] || Key[k]; if(pressKey) { await keyboard.pressKey(pressKey); await delay(PRESS_DELAY); await keyboard.releaseKey(pressKey); await delay(INTERVAL_DELAY); } }
            await delay(100); await keyboard.releaseKey(mKey);
        } else {
            await keyboard.pressKey(mKey); await delay(PRESS_DELAY); await keyboard.releaseKey(mKey); await delay(MENU_OPEN_DELAY); 
            for (let k of sequence) { let pressKey = nutKeyMap[k] || Key[k]; if(pressKey) { await keyboard.pressKey(pressKey); await delay(PRESS_DELAY); await keyboard.releaseKey(pressKey); await delay(INTERVAL_DELAY); } }
        }
    } catch (e) { keyboard.releaseKey(mKey).catch(()=>{}); }
});