# PDFCrop Windows Desktop Experiment

This repository is a **frozen experiment snapshot** based on the upstream `pdfcrop` project. It exists to capture one careful Windows desktop packaging pass, not to replace the original project.

**All real credit belongs to the original authors and contributors.** This snapshot makes no claim of original authorship over the PDF crop engine, the web app, or the project architecture.

## Original upstream projects

This experiment is based on these upstream repositories:

1. [`pdfcrop/pdfcrop`](https://github.com/pdfcrop/pdfcrop) - the main Rust library and CLI
2. [`pdfcrop/pdfcrop.github.io`](https://github.com/pdfcrop/pdfcrop.github.io) - the web app example used by the desktop shell

If you want the original source of truth, history, and ongoing development, those are the repositories to use.

## Authors and attribution

The upstream source and metadata attribute this work to:

1. **Wuqiong Zhao (Teddy van Jerry)** - named author in the main `pdfcrop` crate metadata
2. **pdfcrop contributors** - contributor attribution used throughout the web app and project manifests

This repository is only a packaged experiment around that existing work. It should not be interpreted as transferring authorship or credit away from the upstream project and contributors.

## What this experiment changed

The upstream source tree is preserved under `pdfcrop/`, and this experiment adds only the minimum Windows desktop layer needed to validate the concept:

1. kept the original Rust/WASM crop pipeline,
2. added a Tauri 2 desktop shell,
3. added native desktop open/save behavior,
4. fixed desktop drag and drop on Windows,
5. added a smaller size-focused standalone EXE build profile.

## Important scope note

This is **only an experiment**. It is a frozen snapshot intended to be understandable, reproducible, and respectful to upstream work. It is not an official release branch and not a long-term maintained fork by default.

## Repository layout

- `pdfcrop/` - frozen upstream source snapshot used for the experiment
- `.notes/windows-desktop-conversion-report.md` - concise engineering report for the conversion work

Nested upstream Git histories were removed on purpose so this repository can be pushed as a normal snapshot instead of as embedded repos or submodules.

## How the desktop app works

The desktop app still uses the upstream `pdfcrop` Rust crate compiled to WebAssembly for crop execution. Tauri is used only for:

1. the Windows desktop window,
2. native file dialogs and filesystem access,
3. packaging and distribution.

That means this experiment stayed on the lowest-risk path: preserve the existing crop behavior, change the platform wrapper.

## Fastest path to a Windows EXE

If you already have the required tools installed, these are the exact commands:

```powershell
cd pdfcrop\examples\pdfcrop.github.io
npm install
npm run desktop:build
```

The built app EXE ends up here:

```text
pdfcrop\examples\pdfcrop.github.io\src-tauri\target\release\pdfcrop-desktop.exe
```

The Windows installer EXE ends up here:

```text
pdfcrop\examples\pdfcrop.github.io\src-tauri\target\release\bundle\nsis\PDFCrop_0.1.0_x64-setup.exe
```

For the full beginner-friendly checklist, exact install commands, version checks, and common failure fixes, see [BUILDING.md](BUILDING.md).

## Windows requirements

### To run the built app

1. Windows 10 or Windows 11, 64-bit
2. Microsoft Edge WebView2 Runtime

### To rebuild from source

1. Node.js LTS (`v22` or newer LTS recommended)
2. Rust + rustup
3. Rust target `wasm32-unknown-unknown`
4. `wasm-pack`
5. Tauri CLI 2.x
6. Visual Studio 2022 Build Tools with C++ tooling

## Differences from upstream

Compared with the upstream web example, this snapshot:

1. adds Tauri desktop packaging,
2. adds native desktop file open/save support,
3. enables drag and drop in the Windows desktop app,
4. changes the desktop release profile to reduce EXE size,
5. keeps the original WASM crop engine instead of replacing it with native Tauri crop commands.

## Licensing

Based on upstream metadata and included license text, this experiment is built from permissively licensed upstream code:

1. the main `pdfcrop` crate declares `MIT OR Apache-2.0`,
2. the web example also declares `MIT OR Apache-2.0`,
3. this snapshot includes the upstream MIT license text at the repository root and in the example app directory.

This repository is intentionally prepared without vendored dependency trees, build outputs, installers, or standalone binaries that can be regenerated.
## Desktop usability improvements — draggable & resizable crop corners

- Crop handles were made larger and inset so they remain visible at page edges, improving hit targets for mouse, touch, and pen users.
- The selection overlay now uses PointerEvents (src/js/bbox-overlay.ts) providing unified draw/resize/move modes and robust corner-handle hit-testing.
- This improves usability and accessibility while preserving the existing WASM crop engine and file I/O.
