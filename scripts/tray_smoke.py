"""Isolated Win32 tray lifecycle and real loopback download test.

Only this fixture's process/window handles receive messages. Explorer is never
restarted. Callback and menu selection messages are synthetic; this does not
claim that OS notification banners are visibly delivered under every policy.
"""
import argparse
import ctypes
from ctypes import wintypes as w
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import msvcrt
import os
from pathlib import Path
import sqlite3
import struct
import subprocess
import threading
import time
import uuid

user = ctypes.WinDLL("user32", use_last_error=True)
shell = ctypes.WinDLL("shell32", use_last_error=True)
CALLBACK = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
user.EnumWindows.argtypes = [CALLBACK, w.LPARAM]
user.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
user.GetClassNameW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
user.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
user.IsWindowVisible.argtypes = [w.HWND]
user.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
user.RegisterWindowMessageW.argtypes = [w.LPCWSTR]
class Identifier(ctypes.Structure):
    _fields_ = [("size", w.DWORD), ("hwnd", w.HWND), ("id", w.UINT), ("guid", ctypes.c_byte * 16)]
shell.Shell_NotifyIconGetRect.argtypes = [ctypes.POINTER(Identifier), ctypes.POINTER(w.RECT)]
shell.Shell_NotifyIconGetRect.restype = ctypes.c_long

def windows(pid):
    result = []
    @CALLBACK
    def enum(hwnd, _):
        owner = w.DWORD()
        user.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid:
            name, title = ctypes.create_unicode_buffer(256), ctypes.create_unicode_buffer(256)
            user.GetClassNameW(hwnd, name, 256)
            user.GetWindowTextW(hwnd, title, 256)
            result.append((hwnd, name.value, title.value))
        return True
    assert user.EnumWindows(enum, 0)
    return result

def wait_for(condition, label, timeout=25):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = condition()
        if result:
            return result
        time.sleep(.03)
    raise AssertionError("Timed out: " + label)

def post(hwnd, message, wp=0, lp=0):
    assert user.PostMessageW(hwnd, message, wp, lp), ctypes.get_last_error()

def rect(hwnd, identity):
    identifier = Identifier(ctypes.sizeof(Identifier), hwnd, identity)
    result = w.RECT()
    # Windows returns S_FALSE (1) with the overflow-button rectangle for a hidden icon.
    if shell.Shell_NotifyIconGetRect(ctypes.byref(identifier), ctypes.byref(result)) >= 0:
        return result
    return None

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary-dir", default="target/debug")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    binaries = (root / args.binary_dir).resolve()
    run = root / "artifacts" / ("tray-smoke-" + uuid.uuid4().hex[:8])
    data = run / "data"
    data.mkdir(parents=True)
    database = data / "queue.sqlite3"
    with sqlite3.connect(database) as conn:
        conn.executescript("CREATE TABLE desktop_preferences(id INTEGER PRIMARY KEY,close_to_tray INTEGER NOT NULL,notify_complete INTEGER NOT NULL,notify_failed INTEGER NOT NULL); INSERT INTO desktop_preferences VALUES(1,1,0,0); CREATE TABLE bandwidth_settings(id INTEGER PRIMARY KEY,bytes_per_second INTEGER); INSERT INTO bandwidth_settings VALUES(1,524288);")
    payload = bytes(range(256)) * 16384
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def do_GET(self):
            a, b = 0, len(payload)-1
            range_value = self.headers.get("Range")
            if range_value:
                a,b = map(int, range_value.removeprefix("bytes=").split("-"))
            self.send_response(206 if range_value else 200)
            self.send_header("Content-Length", str(b-a+1))
            self.send_header("ETag", '"tray-fixture-v1"')
            if range_value: self.send_header("Content-Range", f"bytes {a}-{b}/{len(payload)}")
            self.end_headers()
            try: self.wfile.write(payload[a:b+1])
            except (ConnectionResetError, BrokenPipeError): pass
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    env = dict(os.environ, IDM_EX_HOME=str(run))
    for key in ("EFRAME_SCREENSHOT_TO", "IDM_EX_SCREENSHOT_TO", "IDM_EX_TEST_PREFERENCES"):
        env.pop(key, None)
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    log = (run / "desktop.log").open("w", encoding="utf-8")
    process = None
    def launch():
        return subprocess.Popen([str(binaries / "de.exe")], env=env, startupinfo=startup, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
    def handles():
        found = windows(process.pid)
        tray = next((h for h,c,t in found if c == "IDM_EX.NotificationArea.v1"), None)
        main = next((h for h,c,t in found if t == "Download Engine" and c != "IDM_EX.NotificationArea.v1"), None)
        return (main,tray) if main and tray else None
    def add(name):
        request = json.dumps(dict(action="add",request_id=str(uuid.uuid4()),url=f"http://127.0.0.1:{server.server_port}/{name}")).encode()
        host = subprocess.run([str(binaries / "de-host.exe")], input=struct.pack("<I",len(request))+request, env=env, startupinfo=startup, capture_output=True, timeout=15)
        assert host.returncode == 0, host.stderr.decode(errors="replace")
        assert len(host.stdout) == 4 + struct.unpack("<I",host.stdout[:4])[0]
        reply=json.loads(host.stdout[4:]); assert reply["ok"],reply
        return reply["job_id"]
    def job(identity):
        with sqlite3.connect(database) as conn:
            row = conn.execute("SELECT data FROM jobs WHERE id=?",(identity,)).fetchone()
        return json.loads(row[0])
    try:
        process = launch()
        main_window,tray = wait_for(handles,"native windows")
        wait_for(lambda: rect(tray,1),"registered tray icon")
        wait_for(lambda: user.IsWindowVisible(main_window),"initial window visible")
        post(main_window,0x10)
        wait_for(lambda: not user.IsWindowVisible(main_window),"close to tray")
        assert process.poll() is None
        second = subprocess.run([str(binaries / "de.exe")], env=env, startupinfo=startup, capture_output=True, timeout=15)
        assert second.returncode == 0
        wait_for(lambda: user.IsWindowVisible(main_window),"second launch restores existing window")
        post(main_window,0x10)
        wait_for(lambda: not user.IsWindowVisible(main_window),"second hide")
        first = add("hidden-complete.bin")
        wait_for(lambda: job(first)["status"] == "complete", "download completes while hidden")
        assert not user.IsWindowVisible(main_window)
        output=Path(job(first)["saved_path"])
        assert output.read_bytes() == payload
        assert "ZoneId=3" in Path(str(output)+":Zone.Identifier").read_text()
        # Send Explorer's documented recovery message only to our fixture window.
        post(tray,user.RegisterWindowMessageW("TaskbarCreated"))
        wait_for(lambda: rect(tray,2),"tray re-registration")
        assert rect(tray,1) is None
        post(tray,0x8001,0,(1<<16)|0x400) # Retired callback must not restore the app.
        deadline=time.monotonic()+.4
        while time.monotonic()<deadline:
            assert not user.IsWindowVisible(main_window)
            time.sleep(.02)
        post(tray,0x8001,0,(2<<16)|0x400)
        wait_for(lambda: user.IsWindowVisible(main_window),"tray open callback")
        second_job=add("pause-from-tray.bin")
        wait_for(lambda: job(second_job)["downloaded"]>=65536,"active second download")
        post(tray,0x111,2) # Pause all downloads menu item.
        wait_for(lambda: job(second_job)["status"]=="paused","tray pause saves partial data")
        assert job(second_job)["downloaded"]>0
        third=add("exit-from-tray.bin")
        wait_for(lambda: job(third)["downloaded"]>=65536,"active third download")
        post(main_window,0x10)
        wait_for(lambda: not user.IsWindowVisible(main_window),"hide before explicit exit")
        post(tray,0x111,4)
        assert process.wait(timeout=15)==0
        assert job(third)["status"]=="paused" and job(third)["downloaded"]>0
        assert rect(tray,2) is None
        with (data / "desktop.lock").open("r+b") as lock:
            msvcrt.locking(lock.fileno(),msvcrt.LK_NBLCK,1)
            msvcrt.locking(lock.fileno(),msvcrt.LK_UNLCK,1)
        # Default close exits safely even if the native tray context menu is open.
        with sqlite3.connect(database) as conn:
            conn.execute("UPDATE desktop_preferences SET close_to_tray=0")
        process=launch()
        main_window,tray=wait_for(handles,"relaunch after exit")
        icon=wait_for(lambda:rect(tray,1),"new tray icon")
        wait_for(lambda:user.IsWindowVisible(main_window),"relaunch visible")
        post(tray,0x8001,((icon.top & 0xffff)<<16)|(icon.left & 0xffff),(1<<16)|0x7b)
        wait_for(lambda:any(c=="#32768" for _,c,_ in windows(process.pid)),"native tray menu open")
        post(main_window,0x10)
        assert process.wait(timeout=15)==0
        assert rect(tray,1) is None
        result=dict(close_to_tray=True, hidden_transfer_bytes=len(payload), sha256=hashlib.sha256(payload).hexdigest(), internet_origin_marker=True, second_launch_restores=True, tray_open=True, tray_pause=True, explicit_exit_pauses=True, partial_files_retained=True, desktop_lock_released=True, recovery_message_to_own_window=True, stale_callback_ignored=True, open_menu_shutdown=True, tray_icons_removed=True, default_close_exits=True, callbacks="synthetic Win32 messages to fixture-owned windows", notifications="disabled in lifecycle fixture; policy covered by Rust tests",data_root=str(run))
        (run/"result.json").write_text(json.dumps(result,indent=2),encoding="utf-8")
        print(json.dumps(result))
    finally:
        if process and process.poll() is None:
            for hwnd,klass,_ in windows(process.pid):
                if klass=="IDM_EX.NotificationArea.v1":post(hwnd,0x111,4)
            try: process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate();process.wait(timeout=5)
        server.shutdown()
        log.close()

if __name__ == "__main__": main()
