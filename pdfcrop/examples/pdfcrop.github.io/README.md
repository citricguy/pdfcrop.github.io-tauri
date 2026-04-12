# pdfcrop Web App

This example now supports both the original browser workflow and a cross-platform desktop shell built with Tauri.

A WebAssembly-powered PDF cropping tool that runs entirely in your browser. No server uploads, 100% private and secure.

> **Note**: This web app lives in the `examples/pdfcrop.github.io/` directory of the main [pdfcrop](https://github.com/pdfcrop/pdfcrop) Rust project. The build instructions assume this directory structure.

## Features

- 🔒 **100% Private** - All processing happens in your browser via WebAssembly
- 📄 **PDF Viewer** - View and navigate PDFs with zoom and page thumbnails
- ✂️ **Auto-detect** - Automatically detect content boundaries using rendering
- 🎯 **Manual Selection** - Draw custom crop regions per page
- 📏 **Flexible Margins** - Adjust margins (uniform or per-side)
- 📑 **Page Range** - Crop all, odd, even, or custom page ranges
- ⚡ **Fast** - Powered by Rust + WASM
- 🌐 **Works Offline** - No internet required after initial load

## Quick Start

```bash
# From the main pdfcrop repo root
cd examples/pdfcrop.github.io

# Install dependencies
npm install

# Run the web development server
npm run dev

# Build for production
npm run build
```

Visit `http://localhost:8080` to use the app.

## Want the desktop app?

If your goal is to build the desktop app from this snapshot on Windows, Linux, or macOS, use the root-level build guide at:

```text
..\..\..\BUILDING.md
```

The short version is:

```bash
cd pdfcrop/examples/pdfcrop.github.io
npm install
npm run desktop:build
```

If Windows PowerShell blocks `npm` with an `npm.ps1` execution-policy error, use `npm.cmd` for the commands you type manually in that shell. The Tauri config itself should stay on plain `npm` for portability.

Your output bundle depends on the OS you build on:

| Host OS | Typical output |
| --- | --- |
| Windows | `src-tauri\target\release\pdfcrop-desktop.exe` plus installers under `bundle\nsis` and `bundle\msi` |
| macOS | `src-tauri/target/release/bundle/macos/PDFCrop.app` and a DMG under `bundle/dmg` |
| Linux | packages under `src-tauri/target/release/bundle/`, typically `appimage`, `deb`, or `rpm` |

## How It Works

1. **Upload PDF** - Drag and drop or select a PDF file
2. **Auto-detect** - Automatically detects content boundaries
3. **Adjust** - Fine-tune margins or draw custom crop regions
4. **Select Pages** - Choose which pages to crop
5. **Download** - Get your cropped PDF instantly

## Architecture

- **Frontend**: TypeScript + Tailwind CSS
- **PDF Rendering**: PDF.js
- **PDF Processing**: Rust `pdfcrop` library compiled to WASM
- **Desktop Shell**: Tauri 2
- **Desktop File I/O**: Native dialog + file-system access through Tauri plugins
- **Build Tool**: Vite

### Desktop crop execution

Desktop mode **still uses the existing Rust/WASM crop pipeline** from the main `pdfcrop` crate. Tauri currently provides the native window plus native file open/save behavior; it does **not** replace the crop engine with custom native Tauri commands in this version.

## Privacy & Security

Your PDF **never leaves your device**. All processing happens locally in your browser using WebAssembly. No server uploads, no tracking, no data collection.

## Development

```bash
# Development server with hot reload
npm run dev

# Build for production
npm run build

# Preview production build
npm run preview

# Run the desktop app in development mode
npm run desktop:dev

# Build the desktop distributables
npm run desktop:build

# Format code
npm run format
```

## Desktop setup

Use the main `pdfcrop` repository with this example checked out at `examples/pdfcrop.github.io`.

For full install commands and version checks, use the root-level `BUILDING.md` guide.

### Prerequisites

1. Git
2. Node.js and npm (Node 22 LTS or newer LTS recommended)
3. Rust + rustup
4. Rust target `wasm32-unknown-unknown`
5. `wasm-pack`
6. Tauri CLI 2.x
7. Host OS native prerequisites:
   - Windows: Visual Studio 2022 Build Tools with C++ tooling and Microsoft Edge WebView2 Runtime
   - macOS: Xcode Command Line Tools
   - Linux: the Tauri system libraries for your distribution

### Development

```bash
# From the main pdfcrop repo root
cd examples/pdfcrop.github.io

# Install frontend dependencies
npm install

# Launch the desktop app
npm run desktop:dev
```

The dev script rebuilds the WASM package first, starts Vite on `127.0.0.1:8080`, and then launches the Tauri window.

## Desktop build and release

```bash
# From examples/pdfcrop.github.io
npm run desktop:build
```

The build produces native host-OS artifacts under `src-tauri/target/release/bundle/`.

- Windows typically produces `msi` and `nsis`
- macOS typically produces `macos` and `dmg`
- Linux typically produces `appimage`, `deb`, or `rpm`

## Current desktop status

- Click-to-browse open works in desktop mode.
- Drag and drop works in desktop mode.
- Native save works in desktop mode.

## Deployment

The web app is automatically deployed to GitHub Pages on every push to the `main` branch via GitHub Actions.

The deployment workflow:
1. Checks out the main `pdfcrop` repository
2. Builds the WASM module using `wasm-pack`
3. Builds the web app with TypeScript and Vite
4. Deploys the `dist` folder to GitHub Pages

**Live Site**: [https://pdfcrop.github.io](https://pdfcrop.github.io)

To manually trigger a deployment, use the "Run workflow" button in the Actions tab.

## License

MIT OR Apache-2.0
