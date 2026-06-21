# HD2 Macro Terminal — Rust Edition

A Windows x64 HD2 Macro Terminal based on Rust + Tauri + Webview2.

## What it does

- Sends configurable Helldivers II stratagem macro sequences through the Windows `SendInput` API.
- Listens for global keyboard, mouse, and wheel bindings through native Windows hooks.
- Provides a tray-resident main window, a transparent overlay, toast notifications, OCR region selection, help, and sponsor windows.
- Runs local PaddleOCR inference from bundled ONNX detection and recognition models; no cloud OCR service is required.
- Supports OCR selection on multiple displays, including high-DPI displays. OCR regions are stored and captured in physical screen pixels.
- Preserves existing settings by copying supported legacy configuration files into the Rust app-data directory on first launch; legacy files are never moved or deleted.
- Builds a Windows x64 NSIS installer.

## Technology

- Rust 2021
- [Tauri 2](https://v2.tauri.app/)
- Windows API bindings via `windows-rs`
- ONNX Runtime via `ort`
- Local PaddleOCR ONNX models
- HTML, CSS, and JavaScript frontend served from `ui/`

## Requirements

- Windows x64
- Rust stable and Microsoft C++ Build Tools
- Node.js and npm
- Microsoft Edge WebView2 Runtime (included with supported Windows versions in most cases)

## Development

```powershell
cd HD2TriggerSnow
npm install
npm run check
npm test
npm run dev
```

Create a production installer with:

```powershell
npm run build
```

The resulting installer is written to:

```text
src-tauri\target\release\bundle\nsis\HD2 Macro Terminal Rust_0.1.0_x64-setup.exe
```

## Project layout

```text
ui/                         Existing application interface and assets
scripts/                    JavaScript regression checks
src-tauri/src/              Rust application backend
src-tauri/resources/models/ Bundled PaddleOCR ONNX models and dictionary
src-tauri/icons/            Windows application icon
```

## Checks

`npm test` runs the OCR matcher regression script and the Rust test suite. The Rust tests cover configuration persistence, OCR preprocessing and decoding, region capture, high-DPI coordinate conversion, and native OCR model loading.

## Notes

- This repository intentionally includes the OCR models so the packaged application can recognize text offline.
- Build artifacts, dependencies, local settings, logs, credentials, and database files are excluded by `.gitignore`.
- The application is currently scoped to Windows x64.

## License

This project is licensed under the GNU Affero General Public License v3.0.
SPDX-License-Identifier: AGPL-3.0-only
See the [LICENSE](LICENSE) file for details.
