const { app, BrowserWindow, ipcMain, Tray, Menu, nativeImage } = require('electron');
const path = require('path');
const fs = require('fs');
const { uIOhook } = require('uiohook-napi'); 
const { keyboard, Key } = require('@nut-tree/nut-js');

let mainWindow;
let overlayWindow; 
let tray = null;
let isQuitting = false; 

keyboard.config.autoDelayMs = 0; 


const uioToChar = {

    30: "KeyA", 48: "KeyB", 46: "KeyC", 32: "KeyD", 18: "KeyE", 33: "KeyF", 34: "KeyG", 35: "KeyH",
    23: "KeyI", 36: "KeyJ", 37: "KeyK", 38: "KeyL", 50: "KeyM", 49: "KeyN", 24: "KeyO", 25: "KeyP",
    16: "KeyQ", 19: "KeyR", 31: "KeyS", 20: "KeyT", 22: "KeyU", 47: "KeyV", 17: "KeyW", 45: "KeyX",
    21: "KeyY", 44: "KeyZ",

    2: "Digit1", 3: "Digit2", 4: "Digit3", 5: "Digit4", 6: "Digit5", 
    7: "Digit6", 8: "Digit7", 9: "Digit8", 10: "Digit9", 11: "Digit0",

    82: "Numpad0", 79: "Numpad1", 80: "Numpad2", 81: "Numpad3", 75: "Numpad4",
    76: "Numpad5", 77: "Numpad6", 71: "Numpad7", 72: "Numpad8", 73: "Numpad9",

    74: "NumpadSubtract", 78: "NumpadAdd", 55: "NumpadMultiply", 3653: "NumpadDivide", 83: "NumpadDecimal", 3612: "NumpadEnter",
    98: "NumpadDivide",

    59: "F1", 60: "F2", 61: "F3", 62: "F4", 63: "F5", 64: "F6",
    65: "F7", 66: "F8", 67: "F9", 68: "F10", 87: "F11", 88: "F12",

    57: "Space", 15: "Tab", 58: "CapsLock", 1: "Escape", 14: "Backspace", 28: "Enter",
    42: "ShiftLeft", 29: "ControlLeft", 56: "AltLeft", 3675: "MetaLeft",
    54: "ShiftRight", 3613: "ControlRight", 3640: "AltRight", 3676: "MetaRight",
    97: "ControlRight", 100: "AltRight",

    12: "Minus", 13: "Equal", 26: "BracketLeft", 27: "BracketRight",
    39: "Semicolon", 40: "Quote", 41: "Backquote", 43: "Backslash",
    51: "Comma", 52: "Period", 53: "Slash",

    57416: "ArrowUp", 57424: "ArrowDown", 57419: "ArrowLeft", 57421: "ArrowRight",
    3665: "PageUp", 3666: "PageDown", 3655: "Home", 3663: "End", 3660: "Insert", 3667: "Delete"
};

const nutKeyMap = {

    'ControlLeft': Key.LeftControl, 'ControlRight': Key.RightControl,
    'ShiftLeft': Key.LeftShift, 'ShiftRight': Key.RightShift,
    'AltLeft': Key.LeftAlt, 'AltRight': Key.RightAlt,
    'MetaLeft': Key.LeftSuper, 'MetaRight': Key.RightSuper, 'OSLeft': Key.LeftSuper, 'OSRight': Key.RightSuper,

    'ArrowUp': Key.Up, 'ArrowDown': Key.Down, 'ArrowLeft': Key.Left, 'ArrowRight': Key.Right,
    'Up': Key.Up, 'Down': Key.Down, 'Left': Key.Left, 'Right': Key.Right,

    'NumpadAdd': Key.Add, 'NumpadSubtract': Key.Subtract, 'NumpadMultiply': Key.Multiply, 'NumpadDivide': Key.Divide,
    'NumpadDecimal': Key.Decimal, 'NumpadEnter': Key.Enter, 

    'Enter': Key.Enter, 'Escape': Key.Escape, 'Backspace': Key.Backspace, 
    'Space': Key.Space, 'Tab': Key.Tab, 'CapsLock': Key.CapsLock,
    'PageUp': Key.PageUp, 'PageDown': Key.PageDown, 'Home': Key.Home, 'End': Key.End, 
    'Insert': Key.Insert, 'Delete': Key.Delete,

    'Minus': Key.Minus, 'Equal': Key.Equal, 'BracketLeft': Key.LeftBracket, 'BracketRight': Key.RightBracket,
    'Semicolon': Key.Semicolon, 'Quote': Key.Quote, 'Backquote': Key.Grave, 'Backslash': Key.Backslash,
    'Comma': Key.Comma, 'Period': Key.Period, 'Slash': Key.Slash
};

function createWindow() {
    mainWindow = new BrowserWindow({
        width: 715, height: 940, minWidth: 710, minHeight: 938,
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
        focusable: false,
        minWidth: 50, minHeight: 50,
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
    overlayWindow.on('show', () => {
        if (mainWindow && !mainWindow.isDestroyed()) {
            mainWindow.webContents.send('overlay-visibility-changed', true);
        }
    });
    overlayWindow.on('hide', () => {
        if (mainWindow && !mainWindow.isDestroyed()) {
            mainWindow.webContents.send('overlay-visibility-changed', false);
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

    const userDataPath = app.getPath('userData');
    ipcMain.handle('load-data', async (event, filename) => {
        const filePath = path.join(userDataPath, filename);
        try {
            if (fs.existsSync(filePath)) {
                const data = fs.readFileSync(filePath, 'utf-8');
                return JSON.parse(data);
            }
        } catch (e) {
            console.error(`读取文件失败 ${filename}:`, e);
        }
        return null; 
    });

    ipcMain.on('save-data', (event, filename, data) => {
        const filePath = path.join(userDataPath, filename);
        try { 
            fs.writeFileSync(filePath, JSON.stringify(data, null, 2), 'utf-8');
        } catch (e) {
            console.error(`保存文件失败 ${filename}:`, e);
        }
    });
    
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
    
    ipcMain.on('resize-overlay', (event, w, h) => {
        if (overlayWindow && !overlayWindow.isDestroyed()) {
            overlayWindow.setResizable(true);
            overlayWindow.setMinimumSize(50, 50);
            overlayWindow.setSize(parseInt(w), parseInt(h));
            overlayWindow.setResizable(false);
        }
    });
    
    ipcMain.on('update-overlay-settings', (event, settings) => {
        if (overlayWindow && !overlayWindow.isDestroyed()) {
            overlayWindow.webContents.send('overlay-settings', settings);
        }
    });

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

    const { menuKey, menuMode, sequence, menuOpenDelay, pressDelay, intervalDelay } = payload;
    
    if (!sequence || sequence.length === 0) return;

    function parseNutKey(kStr) {
        if (!kStr) return null;
        
        if (nutKeyMap[kStr]) return nutKeyMap[kStr];

        if (/^F\d{1,2}$/.test(kStr)) return Key[kStr];

        if (/^Numpad\d$/.test(kStr)) return Key[kStr.replace('Numpad', 'NumPad')];

        if (/^Digit\d$/.test(kStr)) return Key[kStr.replace('Digit', 'Num')];

        if (/^Key[A-Z]$/.test(kStr)) return Key[kStr.replace('Key', '')];

        if (/^[a-zA-Z]$/.test(kStr)) return Key[kStr.toUpperCase()];
        if (/^\d$/.test(kStr)) return Key[`Num${kStr}`];

        return null;
    }

    const mKey = parseNutKey(menuKey) || Key.LeftControl;
    
    const MENU_OPEN_DELAY = Math.max(1, parseInt(menuOpenDelay) || 150);
    const PRESS_DELAY = Math.max(1, parseInt(pressDelay) || 15);
    const INTERVAL_DELAY = Math.max(1, parseInt(intervalDelay) || 15);

    try {
        await keyboard.releaseKey(mKey).catch(() => {});
        await delay(10);

        if (menuMode === 'hold') {
            await keyboard.pressKey(mKey);
        } else {
            await keyboard.pressKey(mKey);
            await delay(PRESS_DELAY + 20); 
            await keyboard.releaseKey(mKey);
        }

        await delay(MENU_OPEN_DELAY); 

        for (const k of sequence) {
            const pressKey = parseNutKey(k);
            if(pressKey) {
                await keyboard.pressKey(pressKey);
                await delay(PRESS_DELAY); 
                await keyboard.releaseKey(pressKey);
                await delay(INTERVAL_DELAY); 
            }
        }

    } catch (e) {
        console.error("Macro execution error:", e);
    } finally {
        if (menuMode === 'hold') {
            await delay(50); 
            await keyboard.releaseKey(mKey).catch(() => {});
        }
    }
});