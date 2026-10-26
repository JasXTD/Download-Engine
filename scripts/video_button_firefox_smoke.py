"""Disposable Firefox profile; fixture site access is pre-granted, not a prompt test."""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import psutil
from marionette_driver.marionette import Marionette
from marionette_driver.addons import Addons

config = json.loads(Path(sys.argv[1]).read_text())
run = Path(config['run'])
profile = run / 'profile'
profile.mkdir()
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    port = sock.getsockname()[1]
(profile/'user.js').write_text(f'user_pref("marionette.port",{port});\nuser_pref("browser.shell.checkDefaultBrowser",false);', encoding='utf-8')
executable = Path(r'C:\Program Files\Mozilla Firefox\firefox.exe')
log = (run/'firefox.log').open('w', encoding='utf-8')
process = subprocess.Popen([str(executable),'--headless','--no-remote','--profile',str(profile),'--marionette','--remote-allow-system-access'], stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
client = Marionette(port=port, startup_timeout=25, socket_timeout=30)
owned = None
def evaluate(script, *args):
    return client.execute_script(script, script_args=args, sandbox=None)
def until(condition, label):
    end = time.monotonic()+20
    while time.monotonic()<end:
        value=condition()
        if value:
            return value
        time.sleep(.08)
    raise AssertionError('Timed out: '+label)
def activate_source(control):
    client.switch_to_window(control)
    client.set_context('chrome')
    client.execute_script('const win=Services.wm.getMostRecentWindow("navigator:browser"); win.gBrowser.selectedTab=Array.from(win.gBrowser.tabs).find(t=>t.linkedBrowser.currentURI.spec===arguments[0]);', script_args=[config['url']])
    client.set_context('content')
def configure(control, enabled):
    activate_source(control)
    result=client.execute_async_script('const [enabled,done]=arguments; browser.runtime.sendMessage({action:"video_button_status"}).then(state=>browser.runtime.sendMessage({action:"configure_video_button",enabled,pattern:state.pattern})).then(done,e=>done({error:e.message}));', script_args=[enabled], sandbox=None)
    assert result.get('ok'),result
try:
    capabilities=client.start_session()
    assert Path(capabilities['moz:profile']).resolve()==profile.resolve()
    owned=psutil.Process(capabilities['moz:processID'])
    assert Path(owned.exe()).resolve()==executable.resolve()
    arguments=owned.cmdline()
    assert Path(arguments[arguments.index('--profile')+1]).resolve()==profile.resolve()
    client.timeout.script=20
    Addons(client).install(config['extension'],temp=True)
    client.set_context('chrome')
    ids=json.loads(client.execute_script('return Services.prefs.getStringPref("extensions.webextensions.uuids");'))
    client.set_context('content')
    client.navigate('moz-extension://'+ids['download-engine@xetro']+'/popup.html')
    control=client.current_window_handle
    client.execute_async_script('''const done=arguments[0];browser.runtime.getBackgroundPage().then(bg=>{
      bg.videoReviewRequests=[];bg.browser.runtime.sendNativeMessage=async(host,request)=>{
        if(request.action!=="review_browser_video")throw Error("Unexpected native action: "+request.action);
        bg.videoReviewRequests.push(request);return {ok:true,version:1,preview_id:request.request_id,message:"Review in Download Engine"};
      };done(true);
    });''',sandbox=None)
    source=client.open(type='tab')['handle']
    client.switch_to_window(source)
    client.navigate(config['url'])
    until(lambda:evaluate('return !!document.querySelector("[data-idm-ex-video-button]");'),'video button')
    assert evaluate('return document.querySelector("[data-idm-ex-video-button]").shadowRoot;') is None
    (run/'on-page-button.png').write_bytes(client.screenshot(format='binary'))
    bounds=evaluate('const r=document.querySelector("video").getBoundingClientRect();return {x:r.right-84,y:r.top-24};')
    client.actions.sequence('pointer','mouse',{'pointerType':'mouse'}).pointer_move(int(bounds['x']),int(bounds['y'])).pointer_down().pointer_up().perform()
    client.switch_to_window(control)
    def native_requests():
        return client.execute_async_script('const done=arguments[0];browser.runtime.getBackgroundPage().then(bg=>done(bg.videoReviewRequests));',sandbox=None)
    requests=until(native_requests,'native review request')
    assert len(requests)==1 and requests[0]['review']['page_url']==config['url'],requests
    assert sorted(item['kind'] for item in requests[0]['review']['items'])==['dash','file','hls']
    assert len(client.window_handles)==2,'Video review must not open a browser tab'
    (run/'native-review.json').write_text(json.dumps(requests[0],indent=2),encoding='utf-8')
    client.switch_to_window(source)
    client.refresh()
    until(lambda:evaluate('return !!document.querySelector("[data-idm-ex-video-button]");'),'persistent button')
    configure(control,False)
    client.switch_to_window(source)
    until(lambda:not evaluate('return !!document.querySelector("[data-idm-ex-video-button]");'),'removed button')
    client.refresh()
    assert not evaluate('return !!document.querySelector("[data-idm-ex-video-button]");')
    configure(control,True)
    client.switch_to_window(source)
    until(lambda:evaluate('return !!document.querySelector("[data-idm-ex-video-button]");'),'re-enabled button')
    print(json.dumps({'browser':'firefox','version':capabilities['browserVersion'],'checks':['on by default with existing site access','site registration','closed shadow button','trusted click sends native review without a browser tab','DASH/HLS are typed stream candidates','reload persistence','disable cleanup','re-enable after explicit off'],'nativeHandoffTested':False,'screenshots':[str(run/'on-page-button.png')]}))
finally:
    try:
        client.delete_session()
    except Exception:
        pass
    if owned and owned.is_running():
        children=owned.children(recursive=True)
        for child in children:
            try: child.terminate()
            except psutil.NoSuchProcess: pass
        try: owned.terminate()
        except psutil.NoSuchProcess: pass
        psutil.wait_procs(children+[owned],timeout=5)
    if process.poll() is None:
        process.terminate()
        process.wait(timeout=5)
    log.close()
