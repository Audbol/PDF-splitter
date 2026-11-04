# PDF Splitter

A fast, good-looking desktop app for splitting PDF files, written in **Rust**
([egui](https://github.com/emilk/egui) + [lopdf](https://github.com/J-F-Liu/lopdf)).
It ships as a **single portable Windows `.exe`**: no installer, no runtime and no DLLs to bring along.

## Features

- **Five split modes**
  - **Every page**: one PDF per page
  - **Every N pages**: chunks of N pages
  - **Equal parts**: N files of (nearly) equal size
  - **Custom ranges**: one file per range, e.g. `1-3, 5, 8-last`
  - **Extract pages**: chosen pages merged into one file, e.g. `1, 4-6, 9`
- Drag and drop a PDF anywhere onto the window, or use **Browse…**
- Live preview of the files that will be created, with input validation as you type
- Light, dark and system themes
- Password-protected PDFs (RC4 and AES) are supported. The output is unencrypted.
- Never overwrites existing files unless you tick the box (it adds ` (2)` etc. instead)
- Zero-padded file names (`report_01.pdf … report_12.pdf`) so they sort correctly
- Splitting runs in the background with a progress bar and **Cancel**. **Open folder** appears when it's done.
- Output files contain only what their pages need. Links to pages that were left out
  are dropped, so you don't get a copy of the whole document in every file.

### Range syntax

| Input             | Meaning                  |
|-------------------|--------------------------|
| `5`               | page 5                   |
| `2-7`             | pages 2 to 7             |
| `8-` / `8-last`   | page 8 to the end        |
| `-4`              | pages 1 to 4             |
| `5-3`             | pages 5, 4, 3 (reversed) |

Separate ranges with commas or semicolons.

## Download the portable EXE

Every push builds `PDF-Splitter.exe` on GitHub Actions (**Actions → Build Windows EXE →
Artifacts → PDF-Splitter-windows**). Pushing a tag like `v1.0.0` also attaches the exe to a
GitHub Release.

You can drop a PDF onto `PDF-Splitter.exe` in Explorer to open it directly.

> Windows SmartScreen may warn on first launch because the exe isn't code-signed.
> Click **More info → Run anyway**.

## Build it yourself

Install Rust from <https://rustup.rs>, then:

```bash
cargo build --release
```

On Windows the result is `target\release\PDF-Splitter.exe`. The C runtime is linked
statically (see `.cargo/config.toml`), so the exe runs on any Windows 10/11 machine as is.

Cross-compiling from Linux also works:

```bash
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

On Linux, running the app needs the usual X11/Wayland libraries
(e.g. `libxkbcommon-x11-0`, `libgl1`).

## Tests

```bash
cargo test
```

## Project layout

```
src/split.rs    splitting logic: range parsing, page extraction, writing (no GUI)
src/app.rs      egui user interface
src/main.rs     window setup / entry point
build.rs        embeds the icon and version info into the Windows exe
tests/          integration tests (+ encrypted PDF fixtures)
assets/         app icon
```
