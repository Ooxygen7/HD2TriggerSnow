# ONNX Runtime CPU runtime

- Upstream: https://github.com/microsoft/onnxruntime
- Release: `v1.24.4`
- Asset: `onnxruntime-win-x64-1.24.4.zip`
- Asset SHA-256: `d2319fddfb6ea4db99ccc4b60c85c517bcd855721f5daa6a06d40d7cb2ee2357`
- Bundled DLL SHA-256: `b95efb2113b603bbbf3f191061c5516a871ed546893c820e4f3b7b6c358dbf2a`

The DLL is loaded only when OCR is first requested. It is the official CPU
package, avoiding the DirectML and D3D12 dependencies pulled into the previous
statically linked runtime. The upstream license and third-party notices are
bundled alongside it.
