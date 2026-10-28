"""Install, upgrade, and uninstall the dedicated test build in a fresh workspace folder.

No browser registration is created. Existing native-host registry values are
snapshotted before and after. Run Build-Installer.ps1 -TestBuild
-AllowUnsignedDevelopment first.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
from profile_database import open_profile_database
import struct
import subprocess
import time
import uuid
import winreg

ROOT = Path(__file__).resolve().parents[1]
APP_KEY = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\IDM_EX.DownloadManager.SmokeTest_is1"
VIEWS = (winreg.KEY_WOW64_32KEY, winreg.KEY_WOW64_64KEY)
BROWSER_KEYS = ["Software\\" + part + "\\NativeMessagingHosts\\" + host for part in
    (r"Google\Chrome", r"Microsoft\Edge", "Chromium", "Mozilla")
    for host in ("com.xetro.downloadengine", "com.idmex.downloader")]
# The Firefox registry parent has no additional browser-name segment.

def registry_value(key, name="", view=winreg.KEY_WOW64_64KEY):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key, 0, winreg.KEY_READ | view) as handle:
            return winreg.QueryValueEx(handle, name)
    except FileNotFoundError:
        return None


def browser_snapshot():
    return {f"{view}:{key}": registry_value(key, view=view) for key in BROWSER_KEYS for view in VIEWS}


def hidden():
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    return startup


def close_app(process):
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    def close(hwnd, _):
        pid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if pid.value == process.pid:
            user32.PostMessageW(hwnd, 0x0010, 0, 0)
        return True
    callback = callback_type(close)
    deadline = time.monotonic() + 15
    while process.poll() is None and time.monotonic() < deadline:
        user32.EnumWindows(callback, 0)
        try:
            process.wait(timeout=.2)
        except subprocess.TimeoutExpired:
            pass
    assert process.poll() == 0, "Test app did not shut down normally"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--installer", default="dist/DownloadEngine-1.0.0-TestSetup.exe")
    options = parser.parse_args()
    installer = (ROOT / options.installer).resolve()
    assert installer.name.endswith("-TestSetup.exe"), "Use the isolated test installer identity"
    for view in VIEWS:
        assert registry_value(APP_KEY, "InstallLocation", view) is None, "A previous test install exists; inspect it before retrying"
    before = browser_snapshot()
    run_root = ROOT / "artifacts" / ("installer-smoke-" + uuid.uuid4().hex[:8])
    run_root.mkdir()
    program = run_root / "Program files"
    runtime = run_root / "runtime"
    environment = dict(os.environ, IDM_EX_HOME=str(runtime))
    environment.pop("EFRAME_SCREENSHOT_TO", None)
    checks = []
    def setup(label, destination=program):
        result = subprocess.run([str(installer), "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-",
            "/NOICONS", "/TASKS=", "/NOCLOSEAPPLICATIONS", f"/DIR={destination}", f"/LOG={run_root / (label + '.log')}"],
            env=environment, startupinfo=hidden(), capture_output=True, timeout=60)
        return result.returncode
    def uninstall(label):
        result = subprocess.run([str(program / "unins000.exe"), "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART",
            f"/LOG={run_root / (label + '.log')}"], env=environment, startupinfo=hidden(), capture_output=True, timeout=60)
        return result.returncode
    installed = False
    app = None
    try:
        portable = run_root / "existing-portable"
        portable.mkdir()
        (portable / "portable.json").write_text('{"data":"data","downloads":"downloads"}', encoding="utf-8")
        (portable / "de.exe").write_bytes(b"preserve existing program")
        assert setup("refuse-portable", portable) != 0
        assert (portable / "de.exe").read_bytes() == b"preserve existing program"
        checks.append("portable folder refused without changing its files")
        assert setup("install") == 0
        installed = True
        assert registry_value(APP_KEY, "InstallLocation")[0].rstrip("\\") == str(program)
        assert not (program / "portable.json").exists()
        expected = {"de.exe": ROOT / "target/release/de.exe", "de-host.exe": ROOT / "target/release/de-host.exe",
            "de-cli.exe": ROOT / "target/release/de-cli.exe",
            "LICENSE": ROOT / "LICENSE", "THIRD-PARTY-NOTICES.md": ROOT / "THIRD-PARTY-NOTICES.md",
            "CHANGELOG.md": ROOT / "CHANGELOG.md", "docs/github-release.md": ROOT / "docs/github-release.md",
            "README.md": ROOT / "README.md", "installed.json": ROOT / "installer/installed.json",
            "Register-Browsers.ps1": ROOT / "scripts/Register-Browsers.ps1", "docs/browser-release.md": ROOT / "docs/browser-release.md"}
        for directory in ("docs/licenses", "tools/ffmpeg"):
            for source in (ROOT / directory).rglob("*"):
                if source.is_file():
                    expected[source.relative_to(ROOT).as_posix()] = source
        for browser in ("chrome", "edge", "brave", "firefox"):
            for name in ("core.js", "capture.js", "background.js", "popup.html", "popup.css", "popup.js"):
                expected[f"extensions/{browser}/{name}"] = ROOT / "extension" / name
            variant = "firefox" if browser == "firefox" else "chromium"
            expected[f"extensions/{browser}/manifest.json"] = ROOT / "extension" / f"manifest.{variant}.json"
            expected[f"extensions/{browser}/LICENSE"] = ROOT / "LICENSE"
        def check_payload():
            for name, source in expected.items():
                assert (program / name).read_bytes() == source.read_bytes(), name
        check_payload()
        checks.append(f"installed payload matches {len(expected)} source files including MIT, dependency notices and complete FFmpeg sources")
        # Management failures must not open a queue or launch the desktop.
        invalid = subprocess.run([str(program / "de-host.exe"), "--register-browser", "chrome", "invalid"],
            env=environment, input=b"", capture_output=True, startupinfo=hidden(), timeout=10)
        assert invalid.returncode != 0 and b"32-letter" in invalid.stderr
        assert not (runtime / "data/queue.sqlite3").exists()
        checks.append("invalid registration rejected without creating a download database")
        app = subprocess.Popen([str(program / "de.exe")], env=environment, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, startupinfo=hidden())
        database = runtime / "data/queue.sqlite3"
        deadline = time.monotonic() + 20
        while not database.exists() and time.monotonic() < deadline:
            assert app.poll() is None
            time.sleep(.05)
        assert database.exists()
        assert setup("refuse-running-upgrade") != 0
        assert uninstall("refuse-running-uninstall") != 0
        assert app.poll() is None
        assert (program / "de.exe").read_bytes() == expected["de.exe"].read_bytes()
        close_app(app)
        app = None
        checks.append("running installed app blocks upgrade and uninstall without being killed")
        with open_profile_database(database) as connection:
            connection.execute("CREATE TABLE install_test_sentinel (value TEXT)")
            connection.execute("INSERT INTO install_test_sentinel VALUES ('preserve queue data')")
        sentinel = runtime / "downloads/keep.txt"
        sentinel.write_text("preserve downloaded content", encoding="utf-8")
        untracked = program / "keep-user-file.txt"
        untracked.write_text("preserve untracked content", encoding="utf-8")
        (program / "README.md").write_text("old installed copy", encoding="utf-8")
        assert setup("upgrade") == 0
        check_payload()
        with open_profile_database(database) as connection:
            assert connection.execute("SELECT value FROM install_test_sentinel").fetchone()[0] == "preserve queue data"
        checks.append("upgrade replaces program files and preserves existing queue data")
        # A missing cleanup helper aborts before removing program files.
        helper = program / "de-host.exe"
        held_helper = program / "de-host.held"
        helper.rename(held_helper)
        try:
            assert uninstall("refuse-missing-cleanup") != 0
            assert (program / "de.exe").exists()
        finally:
            held_helper.rename(helper)
        checks.append("cleanup failure aborts uninstall before files are removed")
        smokes = []
        for automatic in (False, True):
            command = [os.sys.executable, "-X", "utf8", str(ROOT / "scripts/desktop_smoke.py"), "--binary-dir", str(program)]
            if automatic:
                command.append("--automatic")
            completed = subprocess.run(command, env=environment, capture_output=True, text=True, timeout=60)
            assert completed.returncode == 0, completed.stdout + completed.stderr
            smokes.append(json.loads(completed.stdout))
        checks.append("installed native manual and automatic downloads and cold startup pass")
        assert uninstall("uninstall") == 0
        installed = False
        assert not (program / "de.exe").exists()
        assert sentinel.read_text(encoding="utf-8") == "preserve downloaded content"
        assert untracked.read_text(encoding="utf-8") == "preserve untracked content"
        with open_profile_database(database) as connection:
            assert connection.execute("SELECT value FROM install_test_sentinel").fetchone()[0] == "preserve queue data"
        for view in VIEWS:
            assert registry_value(APP_KEY, "InstallLocation", view) is None
        assert browser_snapshot() == before
        checks.append("uninstall removes the program and preserves queue downloads untracked files and existing browser mappings")
        report = {"ok": True, "checks": checks, "installer_sha256": hashlib.sha256(installer.read_bytes()).hexdigest(),
            "run_root": str(run_root), "desktop_smokes": smokes, "limitation": "Uses a separate uninstall identity; browser extensions are not installed by this test."}
        (run_root / "result.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(json.dumps(report))
    finally:
        if app is not None and app.poll() is None:
            close_app(app)
        if installed and (program / "unins000.exe").exists():
            result = uninstall("failure-cleanup")
            if result != 0:
                print(f"Test uninstall needs inspection: {program}", file=os.sys.stderr)
        assert browser_snapshot() == before, "Existing browser registrations changed"

if __name__ == "__main__":
    main()
