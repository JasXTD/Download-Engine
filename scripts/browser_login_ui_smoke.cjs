const {chromium}=require('playwright');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'..');
const output=path.join(root,'artifacts','browser-login-ui');
fs.mkdirSync(output,{recursive:true});
(async()=>{
  const browser=await chromium.launch({executablePath:'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',headless:true});
  try {
    const context=await browser.newContext({viewport:{width:800,height:850}});
    await context.addInitScript(()=>{
      window.testState={grant:false,remove:false,requests:[],messages:[]};
      window.chrome={
        permissions:{
          request:async grant=>{testState.requests.push({...grant,userGesture:navigator.userActivation.isActive});return testState.grant;},
          remove:async()=>testState.remove
        },
        runtime:{sendMessage:async message=>{
          testState.messages.push(message);
          if(message.action==='browser_login_get')return{ok:true,url:'https://files.example.test:8443/private/archive.zip',origin:'https://files.example.test:8443',permissionOrigin:'https://files.example.test/*',grant:{permissions:['cookies']}};
          return{ok:true,job_id:'fixture',message:'Added with browser login'};
        }}
      };
    });
    const manifest=JSON.parse(fs.readFileSync(path.join(root,'extension/manifest.chromium.json'),'utf8'));
    await context.route('**/*',async route=>{
      const url=new URL(route.request().url());const name=url.pathname.slice(1);
      if(url.hostname!=='idm-ex.test'||!['login.html','login.js','popup.css','i18n.js','i18n-catalog.js'].includes(name))return route.abort();
      await route.fulfill({status:200,contentType:name.endsWith('html')?'text/html':name.endsWith('js')?'text/javascript':'text/css',
        headers:{'Content-Security-Policy':manifest.content_security_policy.extension_pages},
        body:fs.readFileSync(path.join(root,'extension',name))});
    });
    const page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
    await page.goto('https://idm-ex.test/login.html?id=00000000-0000-4000-8000-000000000001');
    await page.waitForFunction(()=>!document.querySelector('#approve').disabled);
    assert.equal(await page.locator('#origin').textContent(),'https://files.example.test:8443');
    assert.equal(await page.evaluate(()=>testState.requests.length),0);
    assert.equal(await page.evaluate(()=>testState.messages.length),1);
    await page.screenshot({path:path.join(output,'light.png'),fullPage:true});
    await page.emulateMedia({colorScheme:'dark'});await page.screenshot({path:path.join(output,'dark.png'),fullPage:true});
    await page.locator('#approve').click();
    await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('not granted'));
    assert.equal(await page.evaluate(()=>testState.messages.length),1);
    const permission=await page.evaluate(()=>testState.requests[0]);
    assert.deepEqual(permission,{permissions:['cookies'],userGesture:true});
    await page.evaluate(()=>{testState.grant=true;});
    await page.locator('#approve').click();
    await page.waitForFunction(()=>document.querySelector('#approve').textContent==='Added to Download Engine');
    assert.equal(await page.evaluate(()=>testState.messages.at(-1).action),'browser_login_submit');
    assert(await page.locator('#approve').isDisabled());
    await page.locator('#forget').click();
    await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('Could not remove'));
    await page.evaluate(()=>{testState.remove=true;});
    await page.locator('#forget').click();
    await page.waitForFunction(()=>document.querySelector('#status').textContent.startsWith('Browser login access removed for all sites.'));
    assert.deepEqual(errors,[]);
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    const report={ok:true,checks:['review shows exact origin and file','no access requested before consent','denial never submits','permission request keeps user gesture and exact hostname','approval submits once','revocation false result handled','successful revocation shown','light and dark layout','no JS exceptions'],
      limitation:'Real Chromium rendering; browser permission and native APIs simulated.'};
    fs.writeFileSync(path.join(output,'result.json'),JSON.stringify(report,null,2));
    console.log(JSON.stringify({...report,output},null,2));
  }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
