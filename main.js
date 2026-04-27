const { app, BrowserWindow, ipcMain, Tray, Menu, nativeImage, screen } = require('electron');
const path = require('path');
const fs = require('fs');
const { uIOhook } = require('uiohook-napi'); 
const { keyboard, Key } = require('@nut-tree/nut-js');
const screenshot = require('screenshot-desktop');
const { Jimp } = require('jimp');

let mainWindow;
let overlayWindow; 
let ocrSelectWindow;
let toastWindow;
let sponsorWindow;
let ocrHelpWindow;
let tray = null;
let isQuitting = false; 
let paddleOcrService = null;
let paddleOcrPromise = null;
const SPONSOR_URL = 'https://www.yifut.com/paypage/?merchant=a0ccz04gJj%2BJNsdjP9cTbIj2MrN958lGiZ7Ub2SdvLGZ';
const SPONSOR_WINDOW_WIDTH = 525;
const SPONSOR_WINDOW_HEIGHT = 675;
const HELP_WINDOW_WIDTH = 525;
const HELP_WINDOW_HEIGHT = 675;

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
        width: 715, height: 1030, minWidth: 715, minHeight: 1030,
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

function normalizeSponsorUrl(url) {
    try {
        const parsed = new URL(url || SPONSOR_URL);
        if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') return SPONSOR_URL;
        return parsed.toString();
    } catch (e) {
        return SPONSOR_URL;
    }
}

async function openSponsorWindow(url = SPONSOR_URL) {
    if (!mainWindow || mainWindow.isDestroyed()) return false;

    const sponsorUrl = normalizeSponsorUrl(url);
    if (sponsorWindow && !sponsorWindow.isDestroyed()) {
        sponsorWindow.webContents.send('sponsor-url', sponsorUrl);
        sponsorWindow.show();
        sponsorWindow.focus();
        return true;
    }

    sponsorWindow = new BrowserWindow({
        width: SPONSOR_WINDOW_WIDTH,
        height: SPONSOR_WINDOW_HEIGHT,
        minWidth: SPONSOR_WINDOW_WIDTH,
        minHeight: SPONSOR_WINDOW_HEIGHT,
        maxWidth: SPONSOR_WINDOW_WIDTH,
        maxHeight: SPONSOR_WINDOW_HEIGHT,
        frame: false,
        resizable: false,
        title: '感谢您的赞助',
        backgroundColor: '#000000',
        parent: mainWindow,
        webPreferences: {
            preload: path.join(__dirname, 'preload.js'),
            contextIsolation: true,
            nodeIntegration: false,
            webviewTag: true
        }
    });

    sponsorWindow.webContents.on('did-attach-webview', (_event, webContents) => {
        webContents.setWindowOpenHandler(({ url }) => {
            webContents.loadURL(url);
            return { action: 'deny' };
        });
    });
    sponsorWindow.on('closed', () => { sponsorWindow = null; });

    await sponsorWindow.loadFile('sponsor.html');
    if (sponsorWindow && !sponsorWindow.isDestroyed()) {
        sponsorWindow.webContents.send('sponsor-url', sponsorUrl);
    }
    return true;
}

async function openOcrHelpWindow(lang = 'zh') {
    if (!mainWindow || mainWindow.isDestroyed()) return false;
    const helpLang = lang === 'en' ? 'en' : 'zh';

    if (ocrHelpWindow && !ocrHelpWindow.isDestroyed()) {
        ocrHelpWindow.webContents.send('ocr-help-lang', helpLang);
        ocrHelpWindow.show();
        ocrHelpWindow.focus();
        return true;
    }

    ocrHelpWindow = new BrowserWindow({
        width: HELP_WINDOW_WIDTH,
        height: HELP_WINDOW_HEIGHT,
        minWidth: HELP_WINDOW_WIDTH,
        minHeight: HELP_WINDOW_HEIGHT,
        maxWidth: HELP_WINDOW_WIDTH,
        maxHeight: HELP_WINDOW_HEIGHT,
        frame: false,
        resizable: false,
        title: 'OCR Help',
        backgroundColor: '#000000',
        parent: mainWindow,
        webPreferences: {
            preload: path.join(__dirname, 'preload.js'),
            contextIsolation: true,
            nodeIntegration: false
        }
    });

    ocrHelpWindow.on('closed', () => { ocrHelpWindow = null; });

    await ocrHelpWindow.loadFile('ocr-help.html');
    if (ocrHelpWindow && !ocrHelpWindow.isDestroyed()) {
        ocrHelpWindow.webContents.send('ocr-help-lang', helpLang);
    }
    return true;
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

function createToastWindow() {
    const display = screen.getPrimaryDisplay();
    const width = Math.min(760, display.workArea.width - 80);
    const height = 110;
    toastWindow = new BrowserWindow({
        width,
        height,
        x: Math.round(display.workArea.x + (display.workArea.width - width) / 2),
        y: Math.round(display.workArea.y + display.workArea.height - height - 120),
        transparent: true,
        frame: false,
        alwaysOnTop: true,
        skipTaskbar: true,
        resizable: false,
        movable: false,
        focusable: false,
        show: false,
        webPreferences: { preload: path.join(__dirname, 'preload.js'), contextIsolation: true }
    });
    toastWindow.setAlwaysOnTop(true, 'screen-saver');
    toastWindow.setIgnoreMouseEvents(true, { forward: true });
    toastWindow.loadFile('toast.html');
}

function positionToastWindow() {
    if (!toastWindow || toastWindow.isDestroyed()) return;
    const display = screen.getPrimaryDisplay();
    const bounds = toastWindow.getBounds();
    toastWindow.setBounds({
        x: Math.round(display.workArea.x + (display.workArea.width - bounds.width) / 2),
        y: Math.round(display.workArea.y + display.workArea.height - bounds.height - 90),
        width: bounds.width,
        height: bounds.height
    });
}

function getVirtualScreenBounds() {
    const displays = screen.getAllDisplays();
    const left = Math.min(...displays.map(d => d.bounds.x));
    const top = Math.min(...displays.map(d => d.bounds.y));
    const right = Math.max(...displays.map(d => d.bounds.x + d.bounds.width));
    const bottom = Math.max(...displays.map(d => d.bounds.y + d.bounds.height));
    return { x: left, y: top, width: right - left, height: bottom - top };
}

function createOcrSelectWindow() {
    if (ocrSelectWindow && !ocrSelectWindow.isDestroyed()) {
        ocrSelectWindow.close();
    }

    const bounds = getVirtualScreenBounds();
    ocrSelectWindow = new BrowserWindow({
        ...bounds,
        transparent: true,
        frame: false,
        alwaysOnTop: true,
        skipTaskbar: true,
        resizable: false,
        movable: false,
        fullscreenable: false,
        focusable: true,
        webPreferences: { preload: path.join(__dirname, 'preload.js'), contextIsolation: true }
    });
    ocrSelectWindow.setAlwaysOnTop(true, 'screen-saver');
    ocrSelectWindow.loadFile('ocr-select.html');
    ocrSelectWindow.on('closed', () => { ocrSelectWindow = null; });
}

function bufferToArrayBuffer(buffer) {
    return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

function loadPaddleOcrModule() {
    const modulePath = path.join(__dirname, 'node_modules', 'paddleocr', 'dist', 'index.js');
    const source = fs.readFileSync(modulePath, 'utf-8');
    const module = { exports: {} };
    const exports = module.exports;
    const loader = new Function('exports', 'module', source + '\nreturn module.exports;');
    return loader(exports, module);
}

function patchPaddleOcrModule(paddleModule) {
    const { DetectionService } = paddleModule;
    if (!DetectionService || DetectionService.__hd2Patched) return;

    DetectionService.prototype.calculateResizeDimensions = function(image) {
        const maxSideLength = this.options.maxSideLength;
        const { width: srcWidth, height: srcHeight } = image;
        const ratio = srcWidth > srcHeight ? maxSideLength / srcWidth : maxSideLength / srcHeight;
        let dstWidth = Math.floor(srcWidth * ratio);
        let dstHeight = Math.floor(srcHeight * ratio);

        dstWidth = Math.max(Math.floor(dstWidth / 32) * 32, 32);
        dstHeight = Math.max(Math.floor(dstHeight / 32) * 32, 32);

        return {
            srcHeight,
            srcWidth,
            dstHeight,
            dstWidth,
            scaleWidth: dstWidth / srcWidth,
            scaleHeight: dstHeight / srcHeight
        };
    };

    const originalRun = DetectionService.prototype.run;
    DetectionService.prototype.run = async function(image) {
        const boxes = await originalRun.call(this, image);
        if (boxes.length > 0) return boxes;

        return [{
            x: 0,
            y: 0,
            width: image.width,
            height: image.height
        }];
    };

    DetectionService.__hd2Patched = true;
}

async function getPaddleOcrService() {
    if (paddleOcrService) return paddleOcrService;
    if (!paddleOcrPromise) {
        paddleOcrPromise = (async () => {
            const ort = require('onnxruntime-node');
            const createSession = ort.InferenceSession.create.bind(ort.InferenceSession);
            class CompatibleTensor extends ort.Tensor {
                constructor(type, data, dims) {
                    if (type === 'float32') {
                        data = data instanceof Float32Array
                            ? new Float32Array(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength))
                            : Float32Array.from(data);
                    }
                    super(type, data, dims);
                }
            }
            const compatibleOrt = {
                ...ort,
                Tensor: CompatibleTensor,
                InferenceSession: {
                    ...ort.InferenceSession,
                    create: (model, options = {}) => createSession(model, {
                        ...options,
                        enableMemPattern: false,
                        executionMode: 'sequential'
                    })
                }
            };
            const paddleModule = loadPaddleOcrModule();
            patchPaddleOcrModule(paddleModule);
            const { PaddleOcrService } = paddleModule;
            if (!PaddleOcrService) throw new Error('PaddleOCR service failed to load');

            const modelDir = path.join(__dirname, 'models', 'ocr');
            const detectionModel = fs.readFileSync(path.join(modelDir, 'PP-OCRv5_mobile_det_infer.onnx'));
            const recognitionModel = fs.readFileSync(path.join(modelDir, 'PP-OCRv5_mobile_rec_infer.onnx'));
            const dictionary = fs.readFileSync(path.join(modelDir, 'ppocrv5_dict.txt'), 'utf-8')
                .split(/\r?\n/)
                .filter(line => line.length > 0);
            dictionary.unshift('');

            paddleOcrService = await PaddleOcrService.createInstance({
                ort: compatibleOrt,
                detection: {
                    modelBuffer: bufferToArrayBuffer(detectionModel),
                    maxSideLength: 1536,
                    minimumAreaThreshold: 6,
                    textPixelThreshold: 0.25,
                    paddingBoxVertical: 0.45,
                    paddingBoxHorizontal: 0.6
                },
                recognition: {
                    modelBuffer: bufferToArrayBuffer(recognitionModel),
                    charactersDictionary: dictionary,
                    imageHeight: 48
                }
            });
            return paddleOcrService;
        })().catch((err) => {
            paddleOcrPromise = null;
            throw err;
        });
    }
    return paddleOcrPromise;
}

async function recognizeOcrRegion(region) {
    if (!region || !Number.isFinite(region.x) || !Number.isFinite(region.y) || !Number.isFinite(region.width) || !Number.isFinite(region.height)) {
        throw new Error('Invalid OCR region');
    }

    const pngBuffer = await screenshot({ format: 'png' });
    const image = await Jimp.read(pngBuffer);
    const x = Math.max(0, Math.round(region.x));
    const y = Math.max(0, Math.round(region.y));
    const width = Math.min(image.bitmap.width - x, Math.max(1, Math.round(region.width)));
    const height = Math.min(image.bitmap.height - y, Math.max(1, Math.round(region.height)));
    if (width <= 0 || height <= 0) throw new Error('OCR region is outside the screenshot');

    const scale = Math.max(1, Math.min(4, Math.floor(1200 / Math.max(width, height))));
    image
        .crop({ x, y, w: width, h: height })
        .contrast(0.18);

    if (scale > 1) {
        image.resize({ w: width * scale });
    }

    const service = await getPaddleOcrService();
    const bitmap = image.bitmap;
    const input = {
        width: bitmap.width,
        height: bitmap.height,
        data: new Uint8Array(bitmap.data)
    };
    const result = await service.recognize(input, { flatten: true, direct: false });
    console.log('[OCR raw]', result && result.text ? result.text.trim() : '<empty>', 'confidence:', result && result.confidence);
    return {
        text: result && result.text ? result.text.trim() : '',
        confidence: result && Number.isFinite(result.confidence) ? result.confidence : 0
    };
}

app.whenReady().then(() => {
    createWindow();
    createOverlayWindow();
    createToastWindow();
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

    ipcMain.handle('open-sponsor', async (_event, url) => {
        try {
            return await openSponsorWindow(url);
        } catch (e) {
            console.error('打开外部链接失败:', e.message);
            return false;
        }
    });

    ipcMain.on('sponsor-window-close', () => {
        if (sponsorWindow && !sponsorWindow.isDestroyed()) sponsorWindow.close();
    });

    ipcMain.handle('open-ocr-help', async (_event, lang) => {
        try {
            return await openOcrHelpWindow(lang);
        } catch (e) {
            console.error('Open OCR help failed:', e.message);
            return false;
        }
    });

    ipcMain.on('ocr-help-window-close', () => {
        if (ocrHelpWindow && !ocrHelpWindow.isDestroyed()) ocrHelpWindow.close();
    });

    ipcMain.handle('get-app-version', async () => app.getVersion());

    ipcMain.on('toggle-overlay', () => {
        if (overlayWindow && !overlayWindow.isDestroyed()) {
            overlayWindow.isVisible() ? overlayWindow.hide() : overlayWindow.showInactive();
        }
    });

    ipcMain.on('start-ocr-region-select', () => {
        createOcrSelectWindow();
    });

    ipcMain.on('ocr-region-selected', (_event, region) => {
        const bounds = ocrSelectWindow && !ocrSelectWindow.isDestroyed()
            ? ocrSelectWindow.getBounds()
            : getVirtualScreenBounds();
        const normalized = {
            x: Math.round(bounds.x + Math.min(region.startX, region.endX)),
            y: Math.round(bounds.y + Math.min(region.startY, region.endY)),
            width: Math.round(Math.abs(region.endX - region.startX)),
            height: Math.round(Math.abs(region.endY - region.startY))
        };
        if (mainWindow && !mainWindow.isDestroyed()) {
            mainWindow.webContents.send('ocr-region-selected', normalized);
        }
        if (ocrSelectWindow && !ocrSelectWindow.isDestroyed()) ocrSelectWindow.close();
    });

    ipcMain.on('cancel-ocr-region-select', () => {
        if (ocrSelectWindow && !ocrSelectWindow.isDestroyed()) ocrSelectWindow.close();
    });

    ipcMain.handle('recognize-ocr-region', async (_event, region) => {
        try {
            return { ok: true, ...(await recognizeOcrRegion(region)) };
        } catch (e) {
            console.error('OCR error:', e);
            return { ok: false, error: e.message || String(e), text: '', confidence: 0 };
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
    ipcMain.on('show-toast', (_event, payload) => {
        if (toastWindow && !toastWindow.isDestroyed()) {
            positionToastWindow();
            toastWindow.showInactive();
            toastWindow.webContents.send('show-toast', payload);
        }
    });

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

app.on('before-quit', () => {
    if (paddleOcrService) {
        paddleOcrService.destroy().catch(() => {});
        paddleOcrService = null;
    }
    paddleOcrPromise = null;
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
