# PDF Splitter

A small, good-looking desktop app for splitting PDF files, packaged as a **single portable
Windows `.exe`** that needs no installation and no Python.

## Features

- **Five split modes**
  - **Every page**: one PDF per page
  - **Every N pages**: chunks of N pages
  - **Equal parts**: N files of (nearly) equal size
  - **Custom ranges**: one file per range, e.g. `1-3, 5, 8-last`
  - **Extract pages**: chosen pages merged into one file, e.g. `1, 4-6, 9`
- Drag and drop a PDF onto the window, or use **Browse…**
- First-page thumbnail, page count and file size
- Live preview of the files that will be created, with input validation
- Light, dark and system themes
- Password-protected PDFs (asks for the password)
- Never overwrites existing files unless you tick the box (adds ` (2)` etc. instead)
- Zero-padded file names (`report_01.pdf … report_12.pdf`) so they sort correctly
- **Open folder** button when done; splitting runs in the background so the UI stays responsive

### Range syntax

| Input      | Meaning                     |
|------------|-----------------------------|
| `5`        | page 5                      |
| `2-7`      | pages 2 to 7                |
| `8-` / `8-last` | page 8 to the end      |
| `-4`       | pages 1 to 4                |
| `5-3`      | pages 5, 4, 3 (reversed)    |

Separate ranges with commas or semicolons.

## Download the portable EXE

Every push builds `PDF-Splitter.exe` on GitHub Actions (**Actions → Build Windows EXE →
Artifacts → PDF-Splitter-windows**). Pushing a tag like `v1.0.0` also attaches the exe to a
GitHub Release.

You can also drop a PDF onto `PDF-Splitter.exe` in Explorer to open it directly.

> Windows SmartScreen may warn on first launch because the exe isn't code-signed.
> Click **More info → Run anyway**.

## Build it yourself (Windows)

```bat
py -3.12 -m venv .venv
.venv\Scripts\activate
pip install -r requirements-dev.txt
pyinstaller --noconfirm --clean pdf_splitter.spec
```

The result is `dist\PDF-Splitter.exe`.

## Run from source (any OS)

```bash
pip install -r requirements.txt
python run.py [optional.pdf]
```

On Linux you may need your distro's Tk package (e.g. `sudo apt install python3-tk`).

## Tests

```bash
pip install -r requirements-dev.txt
pytest
```

## Project layout

```
pdf_splitter/core.py   splitting logic (no GUI, fully tested)
pdf_splitter/gui.py    CustomTkinter interface
run.py                 entry point
pdf_splitter.spec      PyInstaller one-file build config
scripts/make_icon.py   regenerates assets/icon.ico
.github/workflows/     Windows build + release pipeline
```
