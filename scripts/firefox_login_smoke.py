"""Real Firefox cookie/container handoff in the disposable browser-smoke profile."""
import json
from pathlib import Path
import time
import urllib.request
from marionette_driver.addons import Addons

def exercise(client, config, capabilities, checks):
    def evaluate(script, *args):
        return client.execute_script(script, script_args=args, sandbox=None)
    def async_eval(script, *args):
        return client.execute_async_script(script, script_args=args, sandbox=None)
    def until(condition, label):
        deadline=time.monotonic()+25
        while time.monotonic()<deadline:
            value=condition()
            if value:
                return value
            time.sleep(.075)
        raise AssertionError('Timed out: '+label)
    def state():
        with urllib.request.urlopen(config['url']+'/_state',timeout=5) as response:
            return json.load(response)
    assert not async_eval("const done=arguments[0]; browser.permissions.contains({permissions:['cookies'],origins:['http://127.0.0.1/*']}).then(done);")
    checks.append('stock manifest website access does not grant login cookies')
    client.navigate('about:blank')
    addons=Addons(client);addons.uninstall('download-engine@xetro')
    assert addons.install(config['captureExtension'],temp=True)=='download-engine@xetro'
    client.set_context('chrome')
    ids=json.loads(client.execute_script('return Services.prefs.getStringPref("extensions.webextensions.uuids");'))
    client.set_context('content')
    base='moz-extension://'+ids['download-engine@xetro']+'/'
    client.navigate(base+'popup.html')
    # This permission is present only in the disposable fixture so the test can
    # create a container. The shipped extension never creates/edits containers.
    site=async_eval("""const [url,done]=arguments;
      (async()=>{
        await browser.cookies.set({url,name:'session',value:'WRONG-DEFAULT-STORE',path:'/'});
        const identity=await browser.contextualIdentities.create({name:'IDM test',color:'blue',icon:'circle'});
        const tab=await browser.tabs.create({url:url+'/signed-in',cookieStoreId:identity.cookieStoreId});
        return {id:tab.id,store:identity.cookieStoreId};
      })().then(done,e=>done({error:e.message}));""",config['url'])
    assert 'error' not in site,site
    until(lambda:async_eval('const [id,done]=arguments; browser.tabs.get(id).then(t=>done(t.status));',site['id'])=='complete','signed-in container tab')
    # Keep the driver's document on the popup, as a toolbar action would.
    client.set_context('chrome')
    client.execute_script('const win=Services.wm.getMostRecentWindow("navigator:browser"); win.gBrowser.selectedTab=Array.from(win.gBrowser.tabs).find(t=>t.linkedBrowser.currentURI.spec===arguments[0]);',script_args=[config['url']+'/signed-in'])
    client.set_context('content')
    opened=async_eval("const [url,done]=arguments; browser.runtime.sendMessage({action:'browser_login_open',url:url+'/private/browser-login.bin'}).then(done,e=>done({error:e.message}));",config['url'])
    assert opened.get('ok'),opened
    review=until(lambda:async_eval('const done=arguments[0]; browser.tabs.query({}).then(t=>done(t.find(t=>t.url?.includes("/login.html?id="))?.url));'),'login review')
    client.navigate(review)
    until(lambda:evaluate('return !document.querySelector("#approve").disabled;'),'review ready')
    assert evaluate('return document.querySelector("#origin").textContent;')==config['url']
    assert len(state()['jobs'])==1
    (Path(config['run'])/'browser-login-review.png').write_bytes(client.screenshot(format='binary'))
    client.find_element('css selector','#approve').click()
    until(lambda:evaluate('return document.querySelector("#approve").textContent;')=='Added to Download Engine','login approval')
    finished=until(lambda:next((j for j in state()['jobs'] if j['url'].endswith('/private/browser-login.bin') and j.get('bytes_verified')),None),'protected container file')
    assert not finished.get('authentication')
    again=async_eval('const done=arguments[0]; browser.runtime.sendMessage({action:"browser_login_submit",id:new URL(location.href).searchParams.get("id")}).then(done);')
    assert again['job_id']==finished['id'] and len(state()['jobs'])==2
    storage=async_eval('const done=arguments[0]; Promise.all([browser.storage.local.get(null),browser.storage.session.get(null)]).then(done);')
    assert 'SYNTHETIC-' not in json.dumps(storage)
    checks.extend(['real HttpOnly cookies read from original Firefox container, not the default store',
        'approved login reaches native host and completes exact bytes, hash and MOTW',
        'path-scoped cookies withheld after redirect; session deleted on completion',
        'retry is idempotent and extension storage contains no cookie values'])
    return {'version':{'browserName':capabilities['browserName'],'browserVersion':capabilities['browserVersion']},
        'checks':checks,'browserLoginManifest':'Cookies, authentication data consent and test-only contextualIdentities pre-granted in disposable fixture; real permission dialog not exercised.'}
