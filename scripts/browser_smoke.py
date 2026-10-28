"""Run installed browsers against disposable profiles, an app copy, and loopback files.

Requires Node + Playwright for Chromium, marionette_driver for Firefox. Refuses
existing native-host registrations; registration is removed in finally. No real
browser profile, download history, or desktop queue is used.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
from profile_database import open_profile_database
import subprocess
import sys
import threading
import time
import uuid
import winreg

from tray_smoke import windows, post

ROOT = Path(__file__).resolve().parents[1]
HOST = 'com.xetro.downloadengine'
BROWSERS = {
    'chrome': (r'C:\Program Files\Google\Chrome\Application\chrome.exe', r'Google\Chrome'),
    'edge': (r'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe', r'Microsoft\Edge'),
    'firefox': (r'C:\Program Files\Mozilla Firefox\firefox.exe', r'Mozilla'),
}


def registration_missing(browser):
    for host in (HOST, 'com.idmex.downloader'):
        key = 'Software\\' + BROWSERS[browser][1] + '\\NativeMessagingHosts\\' + host
        for view in (winreg.KEY_WOW64_32KEY, winreg.KEY_WOW64_64KEY):
            try:
                with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key, 0, winreg.KEY_READ | view):
                    return False
            except FileNotFoundError:
                pass
    return True


def close_desktop(executable):
    # The executable is a fresh fixture copy. Resolve process identity before
    # posting a normal close, never target the user's copy by name alone.
    command = "Get-CimInstance Win32_Process -Filter \"Name = 'de.exe'\" | Select-Object ProcessId,ExecutablePath | ConvertTo-Json -Compress"
    query = subprocess.run(['powershell.exe', '-NoProfile', '-Command', command], capture_output=True, text=True, check=True, timeout=15)
    rows = json.loads(query.stdout or '[]')
    if isinstance(rows, dict):
        rows = [rows]
    closed = []
    for row in rows:
        if not row['ExecutablePath'] or Path(row['ExecutablePath']).resolve() != executable:
            continue
        pid = row['ProcessId']
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        handle = kernel.OpenProcess(0x00100000 | 0x1000, False, pid)
        assert handle, 'Cannot verify fixture desktop lifetime'
        try:
            for hwnd, _, title in windows(pid):
                if title == 'Download Engine':
                    post(hwnd, 0x0010)
            assert kernel.WaitForSingleObject(handle, 15000) == 0, 'Fixture desktop did not stop'
            code = wintypes.DWORD()
            assert kernel.GetExitCodeProcess(handle, ctypes.byref(code)) and code.value == 0
        finally:
            kernel.CloseHandle(handle)
        closed.append(pid)
    return closed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser', choices=BROWSERS, required=True)
    parser.add_argument('--browser-login', action='store_true', help='Exercise opt-in cookie handoff with synthetic site cookies')
    parser.add_argument('--page-links', action='store_true', help='Exercise the page picker with stock page-access permissions')
    parser.add_argument('--videos', action='store_true', help='Exercise direct video collection with stock permissions')
    parser.add_argument('--binary-dir', default='dist/DownloadEngine')
    parser.add_argument('--node', default=shutil.which('node'))
    parser.add_argument('--firefox-client-dir', default='artifacts/browser-test-deps')
    args = parser.parse_args()
    if args.videos:
        args.page_links = True
    assert registration_missing(args.browser), 'Disconnect the existing host before this opt-in test; it will not be overwritten.'
    executable = Path(BROWSERS[args.browser][0])
    assert executable.is_file(), f'Browser is not installed: {executable}'
    run = ROOT / 'artifacts' / ('browser-smoke-' + args.browser + '-' + uuid.uuid4().hex[:8])
    app = run / 'app with spaces'
    app.mkdir(parents=True)
    for name in ('de.exe', 'de-host.exe'):
        shutil.copy2(ROOT / args.binary_dir / name, app / name)
    (app / 'portable.json').write_text(json.dumps({'data': 'data', 'downloads': 'downloads'}), encoding='utf-8')
    extension = run / 'extension'
    extension.mkdir()
    packaged_extension = ROOT / args.binary_dir / 'extensions' / args.browser
    extension_source = packaged_extension if packaged_extension.is_dir() else ROOT / 'extension'
    for source in extension_source.iterdir():
        if source.is_file() and not source.name.startswith('manifest.'):
            shutil.copy2(source, extension / source.name)
    manifest_path = extension_source / ('manifest.json' if packaged_extension.is_dir() else ('manifest.firefox.json' if args.browser == 'firefox' else 'manifest.chromium.json'))
    manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
    (extension / 'manifest.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    capture_extension = run / 'capture-extension'
    shutil.copytree(extension, capture_extension)
    # Explicit install-time grant in this disposable test profile. Source and
    # release manifests stay unchanged; OS permission prompts are not tested.
    if 'downloads' not in manifest['permissions']:
        manifest['permissions'].append('downloads')
    if args.browser_login:
        manifest['permissions'].append('cookies')
        manifest['optional_permissions'] = [p for p in manifest['optional_permissions'] if p != 'cookies']
        if args.browser == 'firefox':
            manifest['permissions'].append('contextualIdentities')
            consent = manifest['browser_specific_settings']['gecko']['data_collection_permissions']
            consent['required'].append('authenticationInfo')
            consent['optional'].remove('authenticationInfo')
        else:
            manifest['host_permissions'] = ['http://127.0.0.1/*']
    (capture_extension / 'manifest.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    data = app / 'data'
    data.mkdir()
    database = data / 'queue.sqlite3'
    with open_profile_database(database) as conn:
        conn.executescript('CREATE TABLE desktop_preferences(id INTEGER PRIMARY KEY,close_to_tray INTEGER NOT NULL,notify_complete INTEGER NOT NULL,notify_failed INTEGER NOT NULL); INSERT INTO desktop_preferences VALUES(1,0,0,0);')
    if args.page_links:
        # Initialize only this disposable database through the actual native API.
        import struct
        body = json.dumps({'action':'batch_options'}).encode()
        initialized = subprocess.run([str(app/'de-host.exe')], input=struct.pack('<I',len(body))+body, capture_output=True, timeout=15)
        assert initialized.returncode == 0 and json.loads(initialized.stdout[4:])['ok']
        with open_profile_database(database) as conn:
            conn.execute("INSERT INTO download_queues(id,name,max_active,running) VALUES('page-links-test','Page links test',2,0)")
    payload = bytes(range(256)) * 16384
    requests = []
    def state():
        with open_profile_database(database) as conn:
            if not conn.execute("SELECT count(*) FROM sqlite_master WHERE name='jobs'").fetchone()[0]:
                return {'jobs': [], 'captures': []}
            jobs = [json.loads(row[0]) for row in conn.execute('SELECT data FROM jobs')]
            captures = [dict(zip(('state', 'job_id'), row)) for row in conn.execute('SELECT state,job_id FROM browser_captures')]
        for job in jobs:
            if job['status'] == 'complete':
                saved = Path(job['saved_path']).resolve()
                assert saved.is_relative_to(app / 'downloads'), saved
                assert saved.read_bytes() == payload, 'Downloaded bytes differ'
                assert 'ZoneId=3' in Path(str(saved) + ':Zone.Identifier').read_text(encoding='utf-8')
                assert job['integrity']['actual'] == hashlib.sha256(payload).hexdigest(), 'Automatic SHA-256 missing or incorrect'
                job['bytes_verified'] = True
        batches = []
        if args.page_links:
            with open_profile_database(database) as conn:
                batches = [json.loads(row[0]) for row in conn.execute('SELECT job_ids FROM browser_batches')]
        sessions = None
        if args.browser_login:
            with open_profile_database(database) as conn:
                exists = conn.execute("SELECT count(*) FROM sqlite_master WHERE name='browser_sessions'").fetchone()[0]
                sessions = conn.execute("SELECT count(*) FROM browser_sessions").fetchone()[0] if exists else 0
        video_reviews = []
        if args.videos:
            with open_profile_database(database) as conn:
                exists = conn.execute("SELECT count(*) FROM sqlite_master WHERE name='browser_video_reviews'").fetchone()[0]
                if exists:
                    video_reviews = [json.loads(row[0]) for row in conn.execute('SELECT data FROM browser_video_reviews WHERE data IS NOT NULL')]
        return {'jobs': jobs, 'captures': captures, 'batches': batches, 'browser_sessions': sessions, 'video_reviews': video_reviews}
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass
        def do_GET(self):
            if self.path == '/_state':
                self.reply(json.dumps(state()).encode(), 'application/json')
                return
            if args.browser_login and self.path == '/signed-in':
                body = b'<html><title>Signed-in fixture</title><a href="/private/browser-login.bin">Protected file</a></html>'
                self.send_response(200)
                self.send_header('Content-Type', 'text/html')
                self.send_header('Set-Cookie', 'session=SYNTHETIC-BROWSER-LOGIN; Path=/; HttpOnly; SameSite=Lax')
                self.send_header('Set-Cookie', 'path_only=SYNTHETIC-PATH-COOKIE; Path=/private; HttpOnly')
                self.send_header('Set-Cookie', 'unrelated=SYNTHETIC-UNRELATED; Path=/admin; HttpOnly')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            if args.browser_login and self.path in ('/private/browser-login.bin', '/browser-login.bin'):
                cookies = self.headers.get('Cookie', '')
                authenticated = 'session=SYNTHETIC-BROWSER-LOGIN' in cookies
                scoped = 'path_only=' in cookies
                extra = 'unrelated=' in cookies or 'injected=' in cookies
                requests.append({'path': self.path, 'authenticated': authenticated, 'path_cookie': scoped, 'unrelated_cookie': extra})
                if not authenticated or extra or (self.path == '/browser-login.bin' and scoped):
                    self.send_error(401)
                    return
                if self.path.startswith('/private/'):
                    assert scoped, 'Initial download did not receive its matching path cookie'
                    self.send_response(302)
                    self.send_header('Location', '/browser-login.bin')
                    self.send_header('Set-Cookie', 'injected=SHOULD-NOT-COPY; Path=/')
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
            if args.videos and self.path == '/page-links':
                self.reply(b'''<!doctype html><title>Direct video fixture</title><base href="/files/"><video preload="none" src="clip.mp4"></video><video preload="none"><source src="second.webm" type="video/webm"></video><video preload="none" src="/unselected.mp4"></video><a href="clip.mp4#duplicate">Duplicate</a><a href="guide.pdf">Not video</a><a href="/stream.m3u8">HLS excluded</a><a href="/stream.mpd">DASH excluded</a><a href="blob:https://example.test/generated">Generated</a>''', 'text/html')
                return
            if self.path == '/page-links':
                self.reply(b'''<!doctype html><title>Page link fixture</title><base href="/files/"><a href="guide.pdf" download="guide.pdf">Guide</a><a href="source.zip">Source archive</a><a href="guide.pdf#duplicate">Duplicate guide</a><a href="#section">Same-page anchor</a><a href="javascript:alert(1)">Unsafe</a><a href="https://user:secret@invalid.test/file">Credentials</a><a href="data:text/plain,x">Generated</a><a href="/unselected.txt">Leave this one</a>''', 'text/html')
                return
            if self.path == '/':
                self.reply(b'<html><body><a id="off" href="/off.bin">Capture off</a><a id="auto" href="/auto.bin">Capture on</a><a id="fallback" href="/fallback.bin">Disconnected host</a></body></html>', 'text/html')
                return
            if self.path not in ('/browser-login.bin', '/manual.bin', '/off.bin', '/auto.bin', '/fallback.bin', '/files/guide.pdf', '/files/source.zip', '/unselected.txt', '/files/clip.mp4', '/files/second.webm', '/unselected.mp4'):
                self.send_error(404)
                return
            start, end = 0, len(payload) - 1
            range_header = self.headers.get('Range')
            if range_header:
                bounds = range_header.removeprefix('bytes=').split('-')
                start, end = int(bounds[0]), int(bounds[1] or end)
                if start > end or end >= len(payload):
                    self.send_error(416)
                    return
            requests.append({'path': self.path, 'range': range_header})
            self.send_response(206 if range_header else 200)
            self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Disposition', 'attachment; filename="' + self.path.rsplit('/',1)[-1] + '"')
            self.send_header('Content-Length', str(end-start+1))
            self.send_header('ETag', '"browser-smoke-v1"')
            self.send_header('Accept-Ranges', 'bytes')
            if range_header:
                self.send_header('Content-Range', f'bytes {start}-{end}/{len(payload)}')
            self.end_headers()
            try:
                for offset in range(start, end + 1, 65536):
                    self.wfile.write(payload[offset:min(offset + 65536, end + 1)])
                    self.wfile.flush()
                    if end > start:
                        time.sleep(.02)
            except (ConnectionResetError, BrokenPipeError, ConnectionAbortedError):
                pass
        def do_POST(self):
            if args.page_links and self.path == '/_start-page-queue':
                with open_profile_database(database) as conn:
                    conn.execute("UPDATE download_queues SET running=1 WHERE id='page-links-test'")
                self.reply(b'{}', 'application/json')
            else:
                self.send_error(404)
        def reply(self, data, content_type):
            self.send_response(200)
            self.send_header('Content-Type', content_type)
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    config = {'browserLogin': args.browser_login, 'extensionSource': str(extension_source), 'pageLinks': args.page_links, 'videos': args.videos, 'browser': args.browser, 'executable': str(executable), 'run': str(run), 'extension': str(extension), 'captureExtension': str(capture_extension), 'host': str(app / 'de-host.exe'), 'url': f'http://127.0.0.1:{server.server_port}', 'sha256': hashlib.sha256(payload).hexdigest()}
    (run / 'config.json').write_text(json.dumps(config), encoding='utf-8')
    env = dict(os.environ, IDM_EX_HOME=str(app))
    env.pop('EFRAME_SCREENSHOT_TO', None)
    result = {'browser': args.browser, 'run': str(run), 'ok': False}
    try:
        if args.browser == 'firefox':
            sys.path.insert(0, str((ROOT / args.firefox_client_dir).resolve()))
            from firefox_smoke import exercise
            result.update(exercise(config, env))
        else:
            assert args.node, 'Pass --node or put Node on PATH; Playwright must also be resolvable.'
            with (run / 'driver.log').open('w', encoding='utf-8') as output:
                driver = subprocess.run([args.node, str(ROOT / ('scripts/chromium_page_links_smoke.cjs' if args.page_links else 'scripts/chromium_smoke.cjs')), str(run / 'config.json')], env=env, stdout=output, stderr=subprocess.STDOUT, timeout=180)
            if driver.returncode:
                raise AssertionError((run / 'driver.log').read_text(encoding='utf-8'))
            result.update(json.loads((run / 'browser-result.json').read_text(encoding='utf-8')))
        final = state()
        assert len(final['jobs']) == (0 if args.videos else 2), final
        assert all(job.get('bytes_verified') for job in final['jobs']), final
        if args.browser_login:
            assert not final['captures'] and final['browser_sessions'] == 0, final
            protected = [entry for entry in requests if 'authenticated' in entry]
            assert len(protected) >= 3 and all(entry['authenticated'] and not entry['unrelated_cookie'] for entry in protected)
        elif args.videos:
            assert not final['captures'] and not final['batches'], final
            assert len(final['video_reviews']) == 1 and len(final['video_reviews'][0]['items']) == 5, final
            assert not any(request['path'] in ('/unselected.mp4', '/stream.m3u8', '/stream.mpd') for request in requests)
        elif args.page_links:
            assert not final['captures'] and len(final['batches']) == 1, final
            assert len(final['batches'][0]) == 2
            assert not any(request['path'] in ('/unselected.txt', '/unselected.mp4', '/stream.m3u8', '/stream.mpd') for request in requests)
        else:
            assert [row['state'] for row in final['captures']] == ['committed'], final
        result.update(ok=True, jobs=final['jobs'], capture_receipts=final['captures'], server_requests=requests)
    except BaseException as error:
        result['error'] = str(error)
        raise
    finally:
        cleanup = subprocess.run([str(app / 'de-host.exe'), '--unregister-browser', args.browser], env=env, capture_output=True, text=True, timeout=15)
        result['registration_removed'] = cleanup.returncode == 0 and registration_missing(args.browser)
        result['closed_desktop_pids'] = close_desktop((app / 'de.exe').resolve())
        if result['ok'] and len(result['closed_desktop_pids']) != 1:
            result.update(ok=False, error='Desktop did not outlive its browser')
        server.shutdown()
        server.server_close()
        (run / 'result.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
        print(json.dumps({'ok': result['ok'], 'report': str(run / 'result.json'), 'registration_removed': result['registration_removed']}))
        assert result['registration_removed'], 'Fixture registration cleanup failed'
        assert result['ok'], result.get('error', 'Browser check failed')


if __name__ == '__main__':
    main()
