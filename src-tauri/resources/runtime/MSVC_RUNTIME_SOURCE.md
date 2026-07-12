# Microsoft Visual C++ runtime (x64)

These app-local runtime files come from the Visual Studio 2022 Build Tools
redistributable directory `Microsoft.VC143.CRT`, version `14.44.35211.0`.
Every copied DLL has a valid Microsoft Authenticode signature.

- `vcruntime140.dll`: `d5e4d9a3e835fa679450145d6a7d94e36573a509317111904d9b3712c30d9066`
- `vcruntime140_1.dll`: `1f2d41c4aa5db0bc33ebf7b66d72943a817d7ce6cbe880502a9403823633093f`
- `msvcp140.dll`: `0f885b509a685d2bbfa652fed26b5fb31d88fbdab0a978c641d1c7b8aa460aa9`
- `msvcp140_1.dll`: `bfad5aef4c63a669e3c140655cdfdf395b6c979b400a447bd5dcb65ed8826c3d`

Microsoft deployment guidance:

- https://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files
- https://learn.microsoft.com/cpp/windows/walkthrough-deploying-a-visual-cpp-application-to-an-application-local-folder

Redistribution remains subject to the Visual Studio license terms. The
installed Build Tools `Redist.txt` pointer is bundled as `MSVC-REDIST.txt`.
