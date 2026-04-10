# Windows desktop conversion report

## Snapshot status

- This workspace is prepared as a **frozen snapshot** for a new GitHub repository.
- The kept source tree is `pdfcrop/`.
- The extra standalone clone, nested upstream Git metadata, and generated build outputs were removed.
- The root repo is intended to track plain source files only.

## Upstream source of truth

This experiment is based on:

1. `pdfcrop/pdfcrop`
2. `pdfcrop/pdfcrop.github.io`

The web app continues to live at `pdfcrop/examples/pdfcrop.github.io` inside the main project layout because the WASM wrapper depends on the parent crate via `path = "../.."`.

## What changed

1. Added a Tauri 2 desktop shell under `pdfcrop/examples/pdfcrop.github.io/src-tauri`.
2. Kept the existing Rust/WASM crop pipeline rather than rewriting crop execution as native Tauri commands.
3. Added native desktop open/save behavior through Tauri dialog and filesystem plugins.
4. Fixed Windows dev-mode binding by switching the frontend dev URL to `127.0.0.1`.
5. Fixed Windows desktop drag and drop by disabling Tauri's native drag-drop interception for the app window so the existing HTML5 drop zone can receive dropped files.
6. Added a size-focused release profile for the desktop shell:
   - `strip = true`
   - `opt-level = "s"`
   - `lto = true`
   - `codegen-units = 1`
   - `panic = "abort"`

## Validation outcomes

- Web build: **passed**
- Desktop dev launch: **passed**
- Local PDF open in desktop mode: **passed**
- Drag and drop in desktop mode: **passed**
- Crop operation in desktop mode: **passed**
- Native save in desktop mode: **passed**
- Standalone optimized EXE retest: **passed**
- Windows release build: **passed**

## Release artifact notes

Generated release artifacts are intentionally ignored from Git, but the final validated sizes were:

1. `PDFCrop.exe` - `6,084,096` bytes
2. `PDFCrop_0.1.0_x64-setup.exe` - `3,752,527` bytes
3. `PDFCrop_0.1.0_x64_en-US.msi` - `4,317,184` bytes

The optimized standalone EXE was reduced from `12,836,864` bytes to `6,084,096` bytes.

## Licensing and attribution notes

- The main `pdfcrop` crate metadata declares `MIT OR Apache-2.0`.
- The web app example metadata also declares `MIT OR Apache-2.0`.
- The frozen snapshot preserves upstream attribution and includes the upstream MIT license text.
- No vendored dependency trees or generated artifacts are intended to be committed.

## Remaining follow-up ideas

1. If a future desktop version needs faster startup or lower memory use, crop execution could be moved from WASM to native Tauri Rust commands.
2. Node 22 LTS remains the safer documented baseline even though this experiment also built on Node 21.
