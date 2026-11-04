# PyInstaller spec for a single-file, windowed (no console) portable executable.
# Build with:  pyinstaller pdf_splitter.spec
from PyInstaller.utils.hooks import collect_data_files, collect_dynamic_libs

datas = [("assets", "assets")]
datas += collect_data_files("customtkinter")
datas += collect_data_files("tkinterdnd2")
datas += collect_data_files("pypdfium2_raw")
binaries = collect_dynamic_libs("pypdfium2_raw")

a = Analysis(
    ["run.py"],
    pathex=[],
    binaries=binaries,
    datas=datas,
    hiddenimports=["PIL._tkinter_finder", "tkinterdnd2"],
    excludes=["numpy", "matplotlib", "pytest"],
    noarchive=False,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="PDF-Splitter",
    console=False,
    icon="assets/icon.ico",
    version="version_info.txt",
    upx=False,
    runtime_tmpdir=None,
)
