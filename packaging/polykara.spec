# PyInstaller build: one folder holding PolyKara.exe (window) and polykara-cli.exe (console).
# Run through build-windows.ps1, which first fills packaging/build/bin and packaging/build/defaults.
import os

from PyInstaller.utils.hooks import collect_all

root = os.path.abspath(os.path.join(SPECPATH, ".."))
build = os.path.join(SPECPATH, "build")
datas = [(os.path.join(build, "bin"), "bin"), (os.path.join(build, "defaults"), "defaults"), (os.path.join(root, "polykara", "readings"), os.path.join("polykara", "readings"))]
binaries, hiddenimports = [], ["pipeline", "polykara.gui"]
# Packages with data files or native libraries that plain import analysis misses.
for package in ("faster_whisper", "ctranslate2", "pypinyin", "pykakasi", "ToJyutping", "zhconv", "indic_transliteration", "anyascii"):
    package_datas, package_binaries, package_imports = collect_all(package)
    datas += package_datas
    binaries += package_binaries
    hiddenimports += package_imports

a = Analysis(
    [os.path.join(SPECPATH, "entry.py")],
    pathex=[root],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    excludes=["torch", "tensorflow", "matplotlib", "whisperx"],
)
pyz = PYZ(a.pure)
# Lyrics and titles are printed as they are; UTF-8 mode keeps the console and pipes from failing on them.
options = [("X utf8_mode=1", None, "OPTION"), ("u", None, "OPTION")]
window = EXE(pyz, a.scripts, options, exclude_binaries=True, name="PolyKara", console=False)
console = EXE(pyz, a.scripts, options, exclude_binaries=True, name="polykara-cli", console=True)
COLLECT(window, console, a.binaries, a.datas, name="PolyKara")
