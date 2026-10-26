"""Firefox adapter for browser_smoke.py using Mozilla's Marionette driver."""
import hashlib
import json
from pathlib import Path
import socket
import subprocess
import time
import urllib.request
import psutil

from marionette_driver.marionette import Marionette
from marionette_driver.addons import Addons


def exercise(config, env):
    run = Path(config['run'])
    profile = run / 'profile'
    downloads = run / 'browser-downloads'
    profile.mkdir()
    downloads.mkdir()
    with socket.socket() as port_socket:
        port_socket.bind(('127.0.0.1', 0))
        port = port_socket.getsockname()[1]
    prefs = {'marionette.port': port, 'browser.shell.checkDefaultBrowser': False,
             'browser.startup.page': 0, 'browser.startup.homepage': 'about:blank',
             'browser.download.folderList': 2, 'browser.download.dir': str(downloads),
             'browser.download.useDownloadDir': True, 'browser.download.alwaysOpenPanel': False,
             'browser.helperApps.neverAsk.saveToDisk': 'application/octet-stream'}
    (profile/'user.js').write_text('\n'.join('user_pref('+json.dumps(k)+','+json.dumps(v)+');' for k,v in prefs.items()), encoding='utf-8')
    log = (run/'firefox.log').open('w', encoding='utf-8')
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    process = subprocess.Popen([config['executable'], '--headless', '--no-remote', '--profile', str(profile), '--marionette', '--remote-allow-system-access'], env=env, stdout=log, stderr=log, startupinfo=startup)
    client = Marionette(port=port, startup_timeout=25, socket_timeout=30)
    checks = []
    owned_session = False
    browser_process = None
    trace_handle = None
    def record(message):
        checks.append(message)
        print('Firefox: '+message, flush=True)
    def evaluate(script, *args):
        return client.execute_script(script, script_args=args, sandbox=None)
    def until(condition, label, timeout=25):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            value = condition()
            if value:
                return value
            time.sleep(.05)
        if trace_handle:
            client.switch_to_window(trace_handle)
        else:
            client.navigate(popup_url())
        details = client.execute_async_script('const done=arguments[0]; Promise.all([browser.storage.local.get(null),browser.downloads.search({}),window.fixtureEvents||[]]).then(done);',sandbox=None)
        (run/'timeout-state.json').write_text(json.dumps(details,indent=2),encoding='utf-8')
        raise AssertionError('Timed out: '+label+'; details='+json.dumps(details))
    def state():
        with urllib.request.urlopen(config['url']+'/_state', timeout=5) as response:
            return json.load(response)
    def popup_url():
        client.set_context('chrome')
        ids = json.loads(client.execute_script('return Services.prefs.getStringPref("extensions.webextensions.uuids");'))
        client.set_context('content')
        return 'moz-extension://'+ids['download-engine@xetro']+'/popup.html'
    def click(selector):
        client.find_element('css selector', selector).click()
    def status(text):
        def observed():
            actual = evaluate('return {text:document.querySelector("#status").textContent,error:document.querySelector("#status").dataset.error,pending:document.querySelector("#check").disabled};')
            if actual.get('error') == 'true' and not actual['pending']:
                raise AssertionError('Popup error: '+actual['text'])
            return actual['text'] == text
        return until(observed, text)
    def register():
        subprocess.run([config['host'], '--register-browser', 'firefox', 'download-engine@xetro'], env=env, capture_output=True, check=True, timeout=15)
    def file_complete(name):
        path = downloads/name
        if path.exists() and hashlib.sha256(path.read_bytes()).hexdigest() == config['sha256']:
            return True
        return False
    try:
        capabilities = client.start_session()
        assert Path(capabilities["moz:profile"]).resolve() == profile.resolve()
        # Firefox's Windows launcher can create a different main-process PID.
        # Bind automation to the exact executable and fresh profile instead.
        target = psutil.Process(capabilities["moz:processID"])
        assert Path(target.exe()).resolve() == Path(config['executable']).resolve()
        arguments = target.cmdline()
        profile_index = arguments.index('--profile')
        assert Path(arguments[profile_index + 1]).resolve() == profile.resolve()
        browser_process = target
        owned_session = True
        client.timeout.script = 20
        client.timeout.page_load = 25
        addons = Addons(client)
        assert addons.install(config['extension'], temp=True) == 'download-engine@xetro'
        client.navigate(popup_url())
        assert evaluate('return document.querySelector("#auto-capture").checked;')
        click('#check')
        until(lambda:evaluate('return document.querySelector("#status").dataset.error;')=='true', 'missing native host')
        record('stock extension reports missing host; capture defaults on')
        register()
        click('#check')
        status('Download Engine is connected')
        if config.get('pageLinks'):
            from firefox_page_links_smoke import exercise as page_links
            return page_links(client, config, capabilities)
        client.find_element('css selector', '#url').send_keys(config['url']+'/manual.bin')
        click('#send')
        until(lambda:evaluate('return document.querySelector("#url").value;')=='', 'manual handoff')
        manual = until(lambda:next((j for j in state()['jobs'] if j['status']=='complete' and j['url'].endswith('/manual.bin')), None), 'manual download')
        assert manual['bytes_verified']
        record('stock popup -> native host -> cold desktop startup -> exact file and MOTW')
        repeated = client.execute_async_script('const [request,done]=arguments; browser.runtime.sendMessage(request).then(done,error=>done({error:error.message}));', script_args=[{'action':'add','url':manual['url'],'request_id':manual['request_id']}], sandbox=None)
        assert repeated['job_id'] == manual['id'], repeated
        assert len(state()['jobs']) == 1
        record('repeated browser handoff is idempotent')
        (run/'manual-popup.png').write_bytes(client.screenshot(format='binary'))
        if config.get('browserLogin'):
            from firefox_login_smoke import exercise as browser_login
            return browser_login(client, config, capabilities, checks)
        client.navigate('about:blank')
        addons.uninstall('download-engine@xetro')
        assert addons.install(config['captureExtension'], temp=True) == 'download-engine@xetro'
        popup = popup_url()
        client.navigate(popup)
        click('#auto-capture')
        status('Automatic capture is off.')
        client.navigate(config['url'])
        click('#off')
        until(lambda:file_complete('off.bin'), 'capture-off browser download')
        assert len(state()['jobs']) == 1
        record('capture off keeps real browser download')
        client.navigate(popup)
        click('#auto-capture')
        status('Automatic capture is on for direct file downloads.')
        record('automatic capture enabled')
        evaluate('window.fixtureEvents=[]; browser.downloads.onCreated.addListener(item=>window.fixtureEvents.push({type:"created",item})); browser.downloads.onChanged.addListener(item=>window.fixtureEvents.push({type:"changed",item}));')
        trace_handle=client.current_window_handle
        fixture_handle=client.open(type='tab')['handle']
        client.switch_to_window(fixture_handle)
        client.navigate(config['url'])
        click('#auto')
        auto = until(lambda:next((j for j in state()['jobs'] if j['status']=='complete' and j['url'].endswith('/auto.bin')), None), 'automatic capture')
        assert auto['bytes_verified']
        client.switch_to_window(trace_handle)
        items = client.execute_async_script('const done=arguments[0]; browser.downloads.search({}).then(done,error=>done({error:error.message}));', sandbox=None)
        assert any(d['url'].endswith('/auto.bin') and d['state']=='interrupted' and not d['canResume'] for d in items), items
        capture_result = client.execute_async_script('const done=arguments[0]; browser.storage.local.get(null).then(done);', sandbox=None)
        (run/'capture-observed.json').write_text(json.dumps({'items':items,'captureResult':capture_result},indent=2),encoding='utf-8')
        until(lambda:client.execute_async_script('const done=arguments[0]; browser.storage.local.get("lastResult").then(({lastResult})=>done(lastResult?.ok && lastResult.message==="Download sent to Download Engine."));',sandbox=None),'capture success acknowledgement')
        assert len(state()['jobs']) == 2
        record('automatic capture commits once, cancels browser original, preserves exact bytes and MOTW')
        subprocess.run([config['host'], '--unregister-browser', 'firefox'], env=env, capture_output=True, check=True, timeout=15)
        client.navigate(config['url'])
        click('#fallback')
        until(lambda:file_complete('fallback.bin'), 'disconnected fallback')
        assert len(state()['jobs']) == 2
        record('disconnected native host resumes browser download without adding a desktop job')
        client.navigate(popup)
        until(lambda:evaluate('return document.querySelector("#auto-capture").checked;'), 'saved capture setting restored')
        click('#auto-capture')
        status('Automatic capture is off.')
        record('capture can be disabled after host disconnect')
        return {'version':{'browserName':capabilities['browserName'],'browserVersion':capabilities['browserVersion']}, 'checks':checks, 'automaticCaptureManifest':'downloads granted by shipped manifest; real install permission prompt not exercised'}
    finally:
        try:
            if owned_session and client.session:
                try:
                    client._send_message("Marionette:Quit", {})
                except OSError:
                    # The quit command may close its socket before the reply.
                    pass
                client.delete_session(send_request=False)
        finally:
            try:
                if browser_process is not None:
                    try:
                        browser_process.wait(timeout=15)
                    except psutil.TimeoutExpired:
                        browser_process.terminate()
                        browser_process.wait(timeout=10)
                        raise AssertionError('Firefox did not exit normally')
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=10)
            log.close()
