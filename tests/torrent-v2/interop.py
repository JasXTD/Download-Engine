"""Harmless BEP 52 interoperability fixtures; run only in the network-none Docker lab.

Uses the already reviewed Debian libtorrent 2.0.8 peer. No public trackers or DHT.
"""
import hashlib
import json
import os
import pathlib
import queue
import subprocess
import threading
import time
import libtorrent as lt

ROOT = pathlib.Path('/work/v2-tests') / str(time.time_ns())
ROOT.mkdir(parents=True)
print(str(ROOT),flush=True)
ENGINE = '/work/target/debug/de-torrent'
RESULTS = []


def wait(check, seconds=45):
    until = time.monotonic() + seconds
    last = None
    while time.monotonic() < until:
        last = check()
        if last:
            return last
        time.sleep(0.05)
    raise AssertionError(f'Timed out: {last}')


class Engine:
    def __init__(self, name, extra=(), executable=ENGINE):
        self.log = open(ROOT / (name + '.log'), 'w')
        self.process = subprocess.Popen([executable, '--lab-allow-loopback-peers', '--state-dir', str(ROOT / (name + '-state')), *extra], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, text=True, env={**os.environ, 'DE_TORRENT_DEBUG': '1'})
        self.lines = queue.Queue()
        def read():
            for line in self.process.stdout:
                self.lines.put(line)
            self.lines.put(None)
        threading.Thread(target=read, daemon=True).start()

    def reply(self, cmd, **args):
        self.process.stdin.write(json.dumps(dict(cmd=cmd, **args)) + '\n')
        self.process.stdin.flush()
        line = self.lines.get(timeout=60)
        assert line is not None, f'Engine exited ({self.process.poll()})'
        return json.loads(line)

    def call(self, cmd, **args):
        reply = self.reply(cmd, **args)
        assert reply.get('ok'), reply
        return reply.get('result', reply.get('data', reply))

    def close(self, crash=False):
        if self.process.poll() is None:
            if crash:
                self.process.kill()
            else:
                self.call('exit')
            self.process.wait(timeout=10)
        self.log.close()


def fixture(name, kind, piece=32768, multi=False, size=2097291):
    base = ROOT / (name + '-seed')
    base.mkdir(exist_ok=True)
    items = [('pack/a.bin', 20123), ('pack/b.bin', 1048797), ('pack/c.bin', 20123), ('pack/empty.bin', 0)] if multi else [('sample.bin', size)]
    storage = lt.file_storage()
    for index, (relative, size) in enumerate(items):
        path = base / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        block = bytes((i * 37 + index * 19) % 256 for i in range(16384))
        path.write_bytes((block * ((size + len(block) - 1) // len(block)))[:size])
        storage.add_file(relative, size)
    flags = {'v1': lt.create_torrent.v1_only, 'v2': lt.create_torrent.v2_only, 'hybrid': 0}[kind]
    creator = lt.create_torrent(storage, piece, flags=flags)
    lt.set_piece_hashes(creator, str(base))
    data = creator.generate()
    target = ROOT / (name + '.torrent')
    target.write_bytes(lt.bencode(data))
    return base, target, data


def seed(torrent, folder, port, mode='tcp', encrypted=False):
    settings = {'listen_interfaces': f'127.0.0.1:{port}', 'outgoing_interfaces': '127.0.0.1',
                'enable_dht': False, 'enable_lsd': False, 'enable_upnp': False, 'enable_natpmp': False,
                'enable_incoming_tcp': mode == 'tcp', 'enable_outgoing_tcp': mode == 'tcp',
                'enable_incoming_utp': mode == 'utp', 'enable_outgoing_utp': mode == 'utp',
                'allow_multiple_connections_per_ip': True, 'upload_rate_limit': 524288}
    if encrypted:
        settings.update(in_enc_policy=int(lt.enc_policy.forced), out_enc_policy=int(lt.enc_policy.forced), allowed_enc_level=int(lt.enc_level.rc4), prefer_rc4=True)
    session = lt.session(settings)
    params = lt.add_torrent_params()
    params.ti = lt.torrent_info(str(torrent))
    params.save_path = str(folder)
    params.flags &= ~(lt.torrent_flags.paused | lt.torrent_flags.auto_managed)
    params.flags |= lt.torrent_flags.disable_dht | lt.torrent_flags.disable_lsd | lt.torrent_flags.disable_pex
    handle = session.add_torrent(params)
    wait(lambda: handle.status().is_seeding)
    return session, handle


def stats(engine, name):
    result = engine.call('stats', name=name)
    return result.get('torrents', {}).get(name, result)


def snapshot(folder):
    return {str(p.relative_to(folder)): hashlib.sha256(p.read_bytes()).hexdigest() for p in folder.rglob('*') if p.is_file()}


def check_files(engine, name, seed_folder, output, only=None):
    details = engine.call('files', name=name)
    expected = {}
    for entry in details['files']:
        if entry['virtual'] or only is not None and entry['index'] not in only:
            continue
        relative = entry['saved_as']
        candidates = [seed_folder / entry['path'], seed_folder / details['name'] / entry['path']]
        source = next(p for p in candidates if p.exists())
        expected[relative] = hashlib.sha256(source.read_bytes()).hexdigest()
    assert snapshot(output) == expected, (snapshot(output), expected)
    return len(expected)


def transfer(name, kind, port, multi=False, magnet=False, mode='tcp', encrypted=False, recovery=False, selection=False, piece=32768, size=2097291):
    source, torrent, data = fixture(name, kind, piece, multi, size)
    session, handle = seed(torrent, source, port, mode, encrypted)
    extra = ['--no-upload', '--no-seed']
    if recovery: extra += ['--download-limit', '262144']
    if mode == 'utp': extra += ['--utp']
    if encrypted: extra += ['--peer-encryption', 'required']
    engine = Engine(name, extra)
    output = ROOT / (name + '-out')
    output.mkdir(exist_ok=True)
    args = dict(name=name, output=str(output), peers=[f'127.0.0.1:{port}'])
    if magnet:
        args['magnet'] = lt.make_magnet_uri(lt.torrent_info(str(torrent)))
    else:
        args['torrent'] = str(torrent)
    selected = None
    try:
        if selection:
            details = engine.call('files', torrent=str(torrent))
            selected = [e['index'] for e in details['files'] if not e['virtual'] and (e['path'].endswith('b.bin') or e['len'] == 0)]
            args['only'] = selected
        engine.call('add', **args)
        retained = 0
        if recovery:
            def progress():
                state = stats(engine, name)
                return state if 0 < state['progress_bytes'] < state['total_bytes'] else None
            state = wait(progress)
            retained = state['progress_bytes']
            engine.call('pause', name=name)
            stopped = snapshot(output)
            time.sleep(0.3)
            assert snapshot(output) == stopped
            engine.call('resume', name=name)
            wait(lambda: stats(engine, name)['progress_bytes'] > retained)
            engine.close(crash=True)
            session.pause()
            engine = Engine(name, extra)
            engine.call('add', **args)
            wait(lambda: stats(engine, name)['progress_bytes'] >= retained)
            engine.call('stop_all')
            stopped = snapshot(output)
            time.sleep(0.3)
            assert snapshot(output) == stopped
            session.resume()
            engine.call('resume_all')
        wait(lambda: stats(engine, name)['finished'])
        count = check_files(engine, name, source, output, selected)
        assert stats(engine, name).get('uploaded_bytes', 0) == 0
        RESULTS.append(dict(case=name, passed=True, files=count, retained_bytes=retained))
        print(json.dumps(RESULTS[-1]), flush=True)
    finally:
        engine.close()
        session.pause()


def seed_interop(kind, port, piece=32768, magnet=False):
    name = kind + '-seed-' + str(piece) + ('-magnet' if magnet else '')
    source, torrent, data = fixture(name, kind, piece)
    engine = Engine(name, ['--port', str(port), '--upload-limit', '524288'])
    output = ROOT / (name + '-receiver')
    output.mkdir()
    receiver = lt.session({'listen_interfaces': '127.0.0.1:0', 'outgoing_interfaces': '127.0.0.1', 'enable_dht': False, 'enable_lsd': False, 'enable_upnp': False, 'enable_natpmp': False})
    try:
        engine.call('add', name=name, torrent=str(torrent), output=str(source), seed_only=True)
        wait(lambda: stats(engine, name)['finished'])
        params = lt.parse_magnet_uri(lt.make_magnet_uri(lt.torrent_info(str(torrent)))) if magnet else lt.add_torrent_params()
        if not magnet:
            params.ti = lt.torrent_info(str(torrent))
        params.save_path = str(output)
        params.flags &= ~(lt.torrent_flags.paused | lt.torrent_flags.auto_managed)
        params.flags |= lt.torrent_flags.disable_dht | lt.torrent_flags.disable_lsd | lt.torrent_flags.disable_pex
        handle = receiver.add_torrent(params)
        handle.connect_peer(('127.0.0.1', port))
        wait(lambda: handle.status().is_seeding, 60)
        assert snapshot(source) == snapshot(output)
        engine.call('stop_all')
        RESULTS.append(dict(case=name, passed=True, files=len(snapshot(output))))
        print(json.dumps(RESULTS[-1]), flush=True)
    finally:
        engine.close()
        receiver.pause()


def old_client_interop(port, old_seed, magnet=False):
    name = 'hybrid-v1-' + ('seed' if old_seed else 'receiver') + ('-btih-magnet' if magnet else '')
    source, torrent, data = fixture(name, 'hybrid', multi=True)
    output = ROOT / (name + '-out')
    output.mkdir()
    seed_engine = Engine(name + '-seed', ['--port', str(port)], executable='/work/v1-engine' if old_seed else ENGINE)
    receiver = Engine(name + '-receiver', ['--no-seed', '--no-upload'], executable=ENGINE if old_seed else '/work/v1-engine')
    # Download Engine stores multi-file torrent paths relative to the chosen folder.
    source = source / 'pack'
    try:
        seed_engine.call('add', name=name, torrent=str(torrent), output=str(source), seed_only=True)
        wait(lambda: stats(seed_engine, name)['finished'])
        args = {'magnet': 'magnet:?xt=urn:btih:' + str(lt.torrent_info(str(torrent)).info_hashes().v1)} if magnet else {'torrent': str(torrent)}
        receiver.call('add', name=name, output=str(output), peers=[f'127.0.0.1:{port}'], **args)
        wait(lambda: stats(receiver, name)['finished'])
        assert snapshot(source) == snapshot(output)
        RESULTS.append(dict(case=name, passed=True, files=len(snapshot(output))))
        print(json.dumps(RESULTS[-1]), flush=True)
    finally:
        receiver.close()
        seed_engine.close()


def invalid_metadata():
    source, torrent, data = fixture('invalid', 'hybrid', multi=True)
    engine = Engine('invalid', ['--no-upload', '--no-seed'])
    import copy
    try:
        variants = []
        bad = copy.deepcopy(data)
        key = next(iter(bad[b'piece layers']))
        layer = bytearray(bad[b'piece layers'][key]); layer[0] ^= 1
        bad[b'piece layers'][key] = bytes(layer)
        variants.append(('tampered-layer', bad))
        bad = copy.deepcopy(data); bad[b'piece layers'] = {}
        variants.append(('missing-layer', bad))
        bad = copy.deepcopy(data); bad[b'info'][b'meta version'] = 3
        variants.append(('unknown-version', bad))
        bad = copy.deepcopy(data); bad[b'info'][b'file tree'][b'..'] = bad[b'info'][b'file tree'].pop(b'a.bin')
        variants.append(('unsafe-path', bad))
        bad = copy.deepcopy(data); bad[b'info'][b'file tree'][b'a.bin'][b''][b'length'] += 1
        variants.append(('hybrid-file-mismatch', bad))
        bad = copy.deepcopy(data); bad[b'info'][b'file tree'][b'A.bin'] = copy.deepcopy(bad[b'info'][b'file tree'][b'a.bin'])
        variants.append(('case-collision', bad))
        for name, content in variants:
            path = ROOT / (name + '.torrent'); path.write_bytes(lt.bencode(content))
            reply = engine.reply('files', torrent=str(path))
            assert reply.get('ok') is False, (name, reply)
            RESULTS.append(dict(case=name, passed=True, rejected=True))
        print(json.dumps(dict(malformed_metadata_rejected=len(variants))), flush=True)
    finally:
        engine.close()


def web_seed(kind):
    import http.server
    import re
    import urllib.parse
    name = kind + '-web-seed'
    source, torrent, data = fixture(name, kind, multi=True)
    payloads = {'/' + str(p.relative_to(source)): p.read_bytes() for p in source.rglob('*') if p.is_file()}
    requests = []
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args): pass
        def do_GET(self):
            path = urllib.parse.unquote(urllib.parse.urlsplit(self.path).path)
            content = payloads.get(path)
            match = re.fullmatch(r'bytes=(\d+)-(\d+)', self.headers.get('Range', ''))
            if content is None or match is None:
                self.send_error(404); return
            start, end = map(int, match.groups())
            if start > end or end >= len(content):
                self.send_error(416); return
            requests.append(path)
            self.send_response(206)
            self.send_header('Content-Range', f'bytes {start}-{end}/{len(content)}')
            self.send_header('Content-Length', str(end - start + 1))
            self.end_headers()
            self.wfile.write(content[start:end + 1])
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    data[b'url-list'] = [f'http://127.0.0.1:{server.server_port}/'.encode()]
    torrent.write_bytes(lt.bencode(data))
    output = ROOT / (name + '-out'); output.mkdir()
    engine = Engine(name, ['--web-seeds', '--no-upload', '--no-seed'])
    try:
        engine.call('add', name=name, torrent=str(torrent), output=str(output))
        wait(lambda: stats(engine, name)['finished'])
        count = check_files(engine, name, source, output)
        assert requests and not any('.pad/' in path for path in requests)
        RESULTS.append(dict(case=name, passed=True, files=count, http_requests=len(requests)))
        print(json.dumps(RESULTS[-1]), flush=True)
    finally:
        engine.close()
        server.shutdown(); server.server_close(); thread.join(timeout=5)


def main():
    transfer('v1-regression', 'v1', 7601, recovery=True)
    transfer('v2-file', 'v2', 7602, recovery=True)
    transfer('hybrid-file', 'hybrid', 7603, multi=True)
    transfer('v2-magnet', 'v2', 7604, multi=True, magnet=True, selection=True)
    transfer('hybrid-magnet', 'hybrid', 7605, magnet=True, recovery=True)
    transfer('v2-utp-encrypted', 'v2', 7606, mode='utp', encrypted=True, piece=16384)
    transfer('hybrid-encrypted', 'hybrid', 7607, encrypted=True, piece=131072)
    seed_interop('v2', 7608, magnet=True)
    seed_interop('hybrid', 7609, piece=131072, magnet=True)
    old_client_interop(7610, True)
    old_client_interop(7611, False)
    old_client_interop(7612, True, magnet=True)
    invalid_metadata()
    transfer('v2-large-hash-layer', 'v2', 7613, magnet=True, piece=16384, size=8 * 1024 * 1024 + 139)
    web_seed('v2')
    web_seed('hybrid')


if __name__ == '__main__':
    try:
        main()
    finally:
        (ROOT / 'results.json').write_text(json.dumps(RESULTS, indent=2) + '\n')
