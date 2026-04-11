# Windows Build Guide

This guide is for one goal: **start from a fresh Windows machine and end up with a working `.exe` build of this repo**.

If you only want the shortest version, use this once your tools are installed:

```powershell
git clone https://github.com/citricguy/pdfcrop.github.io-tauri.git
cd pdfcrop.github.io-tauri\pdfcrop\examples\pdfcrop.github.io
npm install
npm run desktop:build
```

The main app `.exe` ends up here:

```text
src-tauri\target\release\pdfcrop-desktop.exe
```

The Windows installer `.exe` ends up here:

```text
src-tauri\target\release\bundle\nsis\PDFCrop_0.1.0_x64-setup.exe
```

The MSI installer ends up here:

```text
src-tauri\target\release\bundle\msi\PDFCrop_0.1.0_x64_en-US.msi
```

## What you need installed

Install these in **PowerShell as Administrator**.

After you install something, it is often safest to **close that terminal and open a new one** before running the check commands. New terminals pick up updated `PATH` entries and other environment changes from the installer.

### 1. Git

```powershell
winget install --id Git.Git -e
```

Check it:

```powershell
git --version
```

### 2. Node.js LTS

Use **Node 22 LTS or newer LTS**. Do not use Node 21 for this project. The app installed on Node 21 in testing, but Vite warned that 21 is outside the supported range.

```powershell
winget install --id OpenJS.NodeJS.LTS -e
```

Check it:

```powershell
node -v
npm -v
```

Good result: `node -v` should show `v22...` or `v24...`

If `npm -v` fails in PowerShell with a message about `npm.ps1`, use this instead:

```powershell
npm.cmd -v
```

Some Windows systems block PowerShell script launch by policy. `npm.cmd` usually works without changing that setting.

### 3. Rust and Cargo

```powershell
winget install --id Rustlang.Rustup -e
```

Close PowerShell, open a new one, then check:

```powershell
rustc -V
cargo -V
rustup -V
```

### 4. Rust WebAssembly target

This project builds the PDF crop engine to WebAssembly before Tauri packages the desktop app.

```powershell
rustup target add wasm32-unknown-unknown
```

Check it:

```powershell
rustup target list --installed
```

You should see:

```text
wasm32-unknown-unknown
```

### 5. Visual Studio 2022 Build Tools

This is the Windows native compiler toolchain Rust uses on Windows. Install this **before** `wasm-pack` and before the Tauri CLI, otherwise Cargo may fail with `link.exe not found`.

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

If that command gives you trouble, install it manually from Visual Studio Build Tools and make sure **Desktop development with C++** is included.

After install, close the terminal and open a new one, then check:

```powershell
where.exe link
```

### 6. `wasm-pack`

```powershell
cargo install wasm-pack
```

Check it:

```powershell
wasm-pack --version
```

### 7. Tauri CLI 2.x

This repo uses Tauri 2. Install the Cargo CLI for Tauri 2:

```powershell
cargo install tauri-cli --version "^2"
```

Check it:

```powershell
cargo tauri -V
```

### 8. Microsoft Edge WebView2 Runtime

Tauri apps on Windows need WebView2 to run.

```powershell
winget install --id Microsoft.EdgeWebView2Runtime -e
```

## Fresh clone to EXE

Open a normal PowerShell window and run:

```powershell
git clone https://github.com/citricguy/pdfcrop.github.io-tauri.git
cd pdfcrop.github.io-tauri\pdfcrop\examples\pdfcrop.github.io
npm install
npm run desktop:build
```

That is the exact build flow that succeeded for this repo during verification.

If PowerShell blocks `npm` with an `npm.ps1` execution-policy error, use `npm.cmd` instead:

```powershell
git clone https://github.com/citricguy/pdfcrop.github.io-tauri.git
cd pdfcrop.github.io-tauri\pdfcrop\examples\pdfcrop.github.io
npm.cmd install
npm.cmd run desktop:build
```

## What the build command actually does

When you run:

```powershell
npm run desktop:build
```

it triggers:

1. `cargo tauri build`
2. Tauri runs the app's frontend build first
3. The frontend build runs `wasm-pack build --target web --release --out-dir pkg`
4. TypeScript and Vite build the web UI into `dist`
5. Tauri compiles the Windows desktop app and creates Windows bundles

## Where your built files will be

After a successful build, look here:

### Standalone app EXE

```text
pdfcrop\examples\pdfcrop.github.io\src-tauri\target\release\pdfcrop-desktop.exe
```

This is the raw application executable.

### Installer EXE

```text
pdfcrop\examples\pdfcrop.github.io\src-tauri\target\release\bundle\nsis\PDFCrop_0.1.0_x64-setup.exe
```

This is the normal Windows installer `.exe`.

### MSI installer

```text
pdfcrop\examples\pdfcrop.github.io\src-tauri\target\release\bundle\msi\PDFCrop_0.1.0_x64_en-US.msi
```

## "Did I install everything correctly?" checklist

Run these and make sure they all work:

```powershell
git --version
node -v
npm.cmd -v
rustc -V
cargo -V
rustup target list --installed
where.exe link
wasm-pack --version
cargo tauri -V
```

If any of those fail, fix that tool before trying the build.

## First build expectations

- The first build is slow. Several minutes is normal.
- Rust will compile a lot of dependencies the first time.
- The build may print Rust warnings from the upstream `pdfcrop` crate. Those warnings did **not** block the build in verification.
- Vite may warn about large chunks. That also did **not** block the build in verification.

## If the build fails

### Error about `cl.exe`, linker tools, or MSVC

Visual Studio Build Tools are missing or incomplete. Reinstall them and make sure the C++ workload is included.

### Error about `wasm32-unknown-unknown`

Run:

```powershell
rustup target add wasm32-unknown-unknown
```

### Error that `wasm-pack` is not found

Run:

```powershell
cargo install wasm-pack
```

### Error that `cargo tauri` is not found

Run:

```powershell
cargo install tauri-cli --version "^2"
```

### Node version warning or strange frontend build issues

Use Node LTS:

```powershell
winget install --id OpenJS.NodeJS.LTS -e
```

Then reopen PowerShell and check:

```powershell
node -v
```

## Optional: run the desktop app without making a release build

If you just want to launch it locally in dev mode:

```powershell
cd pdfcrop.github.io-tauri\pdfcrop\examples\pdfcrop.github.io
npm install
npm run desktop:dev
```

## Bottom line

If your goal is simply "build this repo and get a Windows EXE", the only commands you should need after installing the prerequisites are:

```powershell
git clone https://github.com/citricguy/pdfcrop.github.io-tauri.git
cd pdfcrop.github.io-tauri\pdfcrop\examples\pdfcrop.github.io
npm install
npm run desktop:build
```
