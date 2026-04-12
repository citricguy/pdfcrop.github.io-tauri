# Desktop Build Guide (Windows, Linux, macOS)

This guide is for one goal: **build the native Tauri desktop app from this repo on the host OS you are using**.

## Short version

Once your tools are installed, the project build flow is the same on Windows, Linux, and macOS:

```bash
git clone https://github.com/citricguy/pdfcrop.github.io-tauri.git
cd pdfcrop.github.io-tauri/pdfcrop/examples/pdfcrop.github.io
npm install
npm run desktop:build
```

If you are in Windows PowerShell and `npm` fails with an `npm.ps1` execution-policy error, use `npm.cmd` for the commands you type manually in that shell:

```powershell
git clone https://github.com/citricguy/pdfcrop.github.io-tauri.git
cd pdfcrop.github.io-tauri\pdfcrop\examples\pdfcrop.github.io
npm.cmd install
npm.cmd run desktop:build
```

That Windows fallback is only for manual terminal commands. The Tauri config in this repo intentionally uses plain `npm` so the same source tree works on Windows, Linux, and macOS.

## Important host OS note

Tauri desktop bundles are host-platform builds:

| Host OS | Typical release outputs |
| --- | --- |
| Windows | `.exe`, NSIS installer, MSI installer |
| macOS | `.app`, `.dmg` |
| Linux | `AppImage`, `.deb`, `.rpm` depending on installed bundlers |

Build on Windows for Windows bundles, on macOS for macOS bundles, and on Linux for Linux bundles. This guide does not cover cross-compiling desktop installers between operating systems.

## Shared requirements on every OS

Install these first:

1. Git
2. Node.js LTS (`v22` or newer LTS recommended)
3. Rust + rustup
4. Rust target `wasm32-unknown-unknown`
5. `wasm-pack`
6. Tauri CLI 2.x

Check them:

```bash
git --version
node -v
npm -v
rustc -V
cargo -V
rustup -V
rustup target list --installed
wasm-pack --version
cargo tauri -V
```

Good result: `node -v` should show `v22...` or newer LTS.

If `rustup target list --installed` does not include `wasm32-unknown-unknown`, add it:

```bash
rustup target add wasm32-unknown-unknown
```

Install the project-specific Rust tools with:

```bash
cargo install wasm-pack
cargo install tauri-cli --version "^2"
```

## Host OS prerequisites

### Windows

Install the native Windows toolchain and runtime:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Microsoft.EdgeWebView2Runtime -e
```

If the Build Tools installer gives you trouble, install it manually and make sure **Desktop development with C++** is included.

Check the linker:

```powershell
where.exe link
```

### macOS

For desktop-only Tauri builds, install Xcode Command Line Tools:

```bash
xcode-select --install
```

If you choose to install the full Xcode app instead, open it once after installation so it can finish setup.

Check the tools:

```bash
xcode-select -p
clang --version
```

### Linux

Linux needs the Tauri system libraries for your distribution. On Debian or Ubuntu, install:

```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

For Fedora, Arch, openSUSE, NixOS, or other distributions, use the matching package list from the official Tauri prerequisites page:

```text
https://v2.tauri.app/start/prerequisites/
```

## Fresh clone to native desktop build

### Windows PowerShell

```powershell
git clone https://github.com/citricguy/pdfcrop.github.io-tauri.git
cd pdfcrop.github.io-tauri\pdfcrop\examples\pdfcrop.github.io
npm install
npm run desktop:build
```

If PowerShell blocks `npm`, retry those last two commands with `npm.cmd`.

### macOS or Linux shell

```bash
git clone https://github.com/citricguy/pdfcrop.github.io-tauri.git
cd pdfcrop.github.io-tauri/pdfcrop/examples/pdfcrop.github.io
npm install
npm run desktop:build
```

## Optional: run the desktop app in development mode

### Windows PowerShell

```powershell
cd pdfcrop.github.io-tauri\pdfcrop\examples\pdfcrop.github.io
npm install
npm run desktop:dev
```

### macOS or Linux shell

```bash
cd pdfcrop.github.io-tauri/pdfcrop/examples/pdfcrop.github.io
npm install
npm run desktop:dev
```

The dev command rebuilds the WASM package first, starts Vite on `127.0.0.1:8080`, and then launches the Tauri window.

## What the build command does

When you run:

```bash
npm run desktop:build
```

it triggers:

1. `cargo tauri build`
2. Tauri runs the app frontend build first
3. The frontend build runs `wasm-pack build --target web --release --out-dir pkg`
4. TypeScript and Vite build the web UI into `dist`
5. Tauri compiles the native desktop app and creates host-OS bundles

## Where the built files will be

After a successful build, look under `pdfcrop/examples/pdfcrop.github.io/src-tauri/target/release/`.

| Host OS | Main output locations |
| --- | --- |
| Windows | `pdfcrop-desktop.exe`, `bundle/nsis/`, `bundle/msi/` |
| macOS | `bundle/macos/PDFCrop.app`, `bundle/dmg/` |
| Linux | `bundle/appimage/`, `bundle/deb/`, `bundle/rpm/` |

Linux package types depend on what your host system supports. Windows and macOS bundle names also include version and architecture in the generated filenames.

## First build expectations

- The first build is slow. Several minutes is normal.
- Rust will compile a lot of dependencies the first time.
- The build may print Rust warnings from the upstream `pdfcrop` crate without failing the build.
- Vite may warn about large chunks without failing the build.

## If the build fails

### Error about `npm.ps1` on Windows

Use `npm.cmd` for the commands you type in that PowerShell session:

```powershell
npm.cmd install
npm.cmd run desktop:build
```

Do not change `src-tauri/tauri.conf.json` back to `npm.cmd`; plain `npm` is what keeps the Tauri build hooks portable across Windows, Linux, and macOS.

### Error about `cl.exe`, linker tools, or MSVC on Windows

Visual Studio Build Tools are missing or incomplete. Reinstall them and make sure the C++ workload is included.

### Error about missing Xcode tools on macOS

Install or repair Command Line Tools:

```bash
xcode-select --install
```

### Error about missing WebKitGTK or appindicator packages on Linux

Install the Tauri system libraries for your distribution, then rerun the build. Debian and Ubuntu users can use the package list above.

### Error about `wasm32-unknown-unknown`

Run:

```bash
rustup target add wasm32-unknown-unknown
```

### Error that `wasm-pack` is not found

Run:

```bash
cargo install wasm-pack
```

### Error that `cargo tauri` is not found

Run:

```bash
cargo install tauri-cli --version "^2"
```

### Node version warning or strange frontend build issues

Use Node LTS `v22` or newer LTS, then reopen your terminal and check:

```bash
node -v
```

## Bottom line

After the prerequisites are installed, the build command you want is:

```bash
cd pdfcrop.github.io-tauri/pdfcrop/examples/pdfcrop.github.io
npm install
npm run desktop:build
```
