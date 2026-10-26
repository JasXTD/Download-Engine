"""Stock Firefox page picker via real browser action, isolated Marionette session."""
import json
from pathlib import Path
import time
import urllib.request


def exercise(client, config, capabilities):
    checks = []
    def evaluate(script, *args):
        return client.execute_script(script, script_args=args, sandbox=None)
    def until(condition, label):
        end = time.monotonic() + 25
        while time.monotonic() < end:
            result = condition()
            if result:
                return result
            time.sleep(.075)
        raise AssertionError('Timed out: '+label)
    def state():
        with urllib.request.urlopen(config['url']+'/_state', timeout=5) as response:
            return json.load(response)
    def click(selector):
        client.find_element('css selector', selector).click()
    def field(selector, value, event='change'):
        evaluate('const [selector,value,event]=arguments; const el=document.querySelector(selector); el.value=value; el.dispatchEvent(new Event(event,{bubbles:true}));', selector, value, event)
    control = client.current_window_handle
    fixture = client.open(type='tab')['handle']
    client.switch_to_window(fixture)
    client.navigate(config['url']+'/page-links')
    client.switch_to_window(control)
    # Keep Marionette bound to the extension document while making the source
    # page active, as with a toolbar popup attached to that source page.
    client.set_context('chrome')
    client.execute_script('const win=Services.wm.getMostRecentWindow("navigator:browser"); win.gBrowser.selectedTab=Array.from(win.gBrowser.tabs).find(t=>t.linkedBrowser.currentURI.spec===arguments[0]);', script_args=[config['url']+'/page-links'])
    client.set_context('content')
    assert evaluate('return location.pathname;') == '/popup.html'
    denied = client.execute_async_script('const done=arguments[0]; browser.tabs.query({active:true,currentWindow:true}).then(async ([tab])=>{try{await browser.scripting.executeScript({target:{tabId:tab.id},func:()=>document.title});done(false);}catch{done(true);}});', sandbox=None)
    assert denied, 'Stock extension must have no page access before its action'
    checks.append('page access denied before explicit browser action')
    client.set_context('chrome')
    action = client.execute_async_script('''const done=arguments[0];
      const {ExtensionParent}=ChromeUtils.importESModule("resource://gre/modules/ExtensionParent.sys.mjs");
      const extension=WebExtensionPolicy.getByID("download-engine@xetro").extension;
      const win=Services.wm.getMostRecentWindow("navigator:browser");
      Promise.resolve(ExtensionParent.apiManager.global.browserActionFor(extension).triggerAction(win)).then(()=>done({ok:true}),e=>done({error:e.message}));''')
    assert action.get('ok'), action
    client.set_context('content')
    evaluate('document.querySelector(arguments[0]).click();', '#page-videos' if config.get('videos') else '#page-links')
    if config.get('videos'):
        def pending_review():
            value = state()
            return value if len(value['video_reviews']) == 1 else None
        saved = until(pending_review, 'native video review')
        assert not saved['jobs'] and not saved['batches'], saved
        assert sorted(item['kind'] for item in saved['video_reviews'][0]['items']) == ['dash','file','file','file','hls']
        assert len(client.window_handles) == 2, 'Video review must not open a browser tab'
        checks.append('video scan opens native review with typed files and streams; no automatic download')
        return {'version':{'browserName':capabilities['browserName'],'browserVersion':capabilities['browserVersion']},'checks':checks,'nativeSelectionTested':False}
    def scanned():
        result=evaluate('return {text:document.querySelector("#status").textContent,error:document.querySelector("#status").dataset.error};')
        if result['error']=='true':
            raise AssertionError(result['text'])
        return result['text'].startswith('Found ')
    until(scanned,'page scan')
    picker = None
    for handle in client.window_handles:
        client.switch_to_window(handle)
        if evaluate('return location.pathname;') == '/links.html':
            picker = handle
            break
    assert picker, 'Picker did not open'
    until(lambda:evaluate('return document.querySelectorAll("#links tr").length;')==3,'picker links')
    assert evaluate('return document.querySelectorAll("#links input:checked").length;')==0
    assert len(state()['jobs'])==0
    checks.append('stock action resolves relative links, deduplicates, rejects unsafe links and leaves selection empty')
    field('#search','guide','input'); click('#select-visible')
    field('#search','source','input'); click('#select-visible'); field('#search','','input')
    field('#queue','page-links-test'); field('#destination','documents')
    (Path(config['run'])/'page-picker.png').write_bytes(client.screenshot(format='binary'))
    click('#add')
    until(lambda:evaluate('return document.querySelector("#add").textContent;')=='Added to queue','batch receipt')
    saved=state()
    assert len(saved['jobs'])==2 and len(saved['batches'])==1, saved
    assert all(j['status']=='queued' and j['directory'].endswith('Documents') for j in saved['jobs']), saved
    repeat=client.execute_async_script('const done=arguments[0]; browser.runtime.sendMessage({action:"page_links_submit",id:location.hash.slice(1),retry:true}).then(done);',sandbox=None)
    assert repeat['count']==2 and len(state()['jobs'])==2
    client.navigate(evaluate('return location.href;'))
    until(lambda:evaluate('return document.querySelector("#add").textContent;')=='Added to queue','reloaded receipt')
    checks.append('native batch starts cold desktop, respects paused queue and saved folder; repeat and tab reload do not duplicate')
    with urllib.request.urlopen(urllib.request.Request(config['url']+'/_start-page-queue',data=b'',method='POST'),timeout=5):
        pass
    def complete():
        value=state()
        return value if len(value['jobs'])==2 and all(j.get('bytes_verified') for j in value['jobs']) else None
    completed=until(complete,'queue completion')
    assert sorted(j['name'] for j in completed['jobs'])==['guide.pdf','source.zip']
    checks.append('selected files complete with exact bytes and Windows MOTW; unselected link untouched')
    return {'version':{'browserName':capabilities['browserName'],'browserVersion':capabilities['browserVersion']},'checks':checks,'pageAccess':'Unmodified release permissions. Marionette triggers the actual Firefox browser action; popup button script runs in a background extension tab.'}
