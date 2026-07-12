# HELLDIVERS 2 Quick Stratagem Tool

<p align="center">
  <img alt="Rust 2021" src="https://img.shields.io/badge/Rust-2021-000000?logo=rust&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24C8D8?logo=tauri&logoColor=white">
  <img alt="Windows x64" src="https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4?logo=windows11&logoColor=white">
  <img alt="ONNX Runtime" src="https://img.shields.io/badge/ONNX_Runtime-Offline_OCR-005CED?logo=onnx&logoColor=white">
</p>

专为《HELLDIVERS 2》设计的 Windows 战备快捷输入工具。Rust 版本延续原版的操作方式，并进一步改善了启动速度、资源占用、长时间运行稳定性和 OCR 识别体验。

> 本工具不会读取或修改游戏内存，只通过 Windows 标准输入接口发送按键。请自行了解并遵守游戏及平台规则。

## 主要功能

- 为常用战备配置独立快捷键，支持键盘、鼠标按键和滚轮。
- 自由调整战备顺序、锁定固定战备，并保存多套预设。
- 提供可调大小、透明度和按键样式的悬浮窗。
- 支持离线 OCR 自动识别当前战备，无需上传截图或连接云端服务。
- 支持多显示器和高 DPI 屏幕，可精确框选需要识别的区域。
- 常驻系统托盘，保留旧版本配置并在首次启动时自动迁移。
- 主界面采用无边框圆角窗口，保持原版风格并改善显示细节。

## 下载与安装

1. 前往 [Releases](https://github.com/Ooxygen7/HELLDIVERS2_QuickStratagemTool/releases) 下载最新的 Windows x64 安装包。
2. 如果旧版本仍在运行，请先从系统托盘彻底退出。
3. 运行安装程序，完成后启动 **HD2 Macro Terminal**。

程序面向 Windows 10 / 11 x64。大多数系统已经自带 WebView2；如果界面无法打开，请先安装最新版 Microsoft Edge WebView2 Runtime。

## 快速开始

1. 打开“战备库”，将需要的战备加入主界面。
2. 点击战备右侧的绑定按钮，为它设置触发键。
3. 在“游戏设置”中确认呼出战备栏按键、方向键和输入延迟与游戏设置一致。
4. 进入游戏后按下绑定键，即可自动输入对应战备指令。

建议先在安全环境中测试按键与延迟。不同电脑、键盘和游戏帧率可能需要略微调整等待时间。

## OCR 自动装配

1. 在游戏中展开左上角战备列表并保持显示。
2. 打开“游戏设置 → OCR 自动识别”，选择游戏所在屏幕。
3. 点击“框选识别区域”，只框住战备列表中的文字部分。
4. 设置 OCR 快捷键；之后展开战备列表并按下该快捷键即可自动装配识别结果。

OCR 全程在本地运行。框选越准确，识别效果越稳定；黄色任务战备目前不会参与自动识别，建议手动添加并锁定。

## 常见问题

### 快捷键没有反应

请确认工具仍在系统托盘运行，绑定键没有与其他软件冲突，并检查游戏内的战备呼出键和方向键是否与工具设置一致。

### OCR 识别不到或识别错误

重新框选更小、更准确的文字区域，并确认选择了正确的显示器。游戏分辨率、界面缩放、动态模糊和截图清晰度都会影响结果。

### 安装新版后仍然显示旧界面

旧程序可能仍在后台运行。请从系统托盘退出旧版本，再启动新安装的版本。

## 本地构建

需要 Rust stable、Node.js、npm 和 Microsoft C++ Build Tools：

```powershell
npm ci
npm run check
npm test
npm run dev
```

生成 Windows x64 安装包：

```powershell
npm run build
```

安装包会生成在 `src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/`。

## 许可与声明

本项目采用 [GNU Affero General Public License v3.0](LICENSE)（`AGPL-3.0-only`）许可。

本项目是社区工具，与 Arrowhead Game Studios、Sony Interactive Entertainment 或《HELLDIVERS 2》官方没有隶属或授权关系。游戏名称、图标及相关素材的权利归各自权利人所有。
