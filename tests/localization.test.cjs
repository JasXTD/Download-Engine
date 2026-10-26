const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const {spawnSync}=require('node:child_process');
const root=path.resolve(__dirname,'..');
const read=file=>fs.readFileSync(path.join(root,file),'utf8');
const registry=JSON.parse(read('locales/languages.json'));
const languages=registry.map(language=>language.code);
const additional=JSON.parse(read('locales/additional.json'));
const extension=JSON.parse(read('locales/extension.json'));
const desktop=JSON.parse(read('locales/desktop.json'));

test('catalogs retain format placeholders, nonempty translations and referenced labels',()=>{
  for(const [kind,catalog,count] of [['desktop',desktop,3],['extension',extension,3],['additional',additional,languages.length-4]]) {
    const slots=text=>[...text.replaceAll('{{','').replaceAll('}}','').matchAll(/\{([^{}]*)\}/g)].map(m=>m[1]).sort();
    for(const [key,values] of Object.entries(catalog)) {
      assert.equal(values.length,count,key);
      for(const text of values) {
        assert.equal(typeof text,'string',key); assert(text.trim(),key);
        assert.deepEqual(slots(text),slots(key),`${kind}: ${key}`);
        assert(!text.includes('\ufffd'),`Invalid Unicode: ${key}`);
      }
    }
  }
  for(const key of new Set([...Object.keys(desktop),...Object.keys(extension)])) assert(additional[key],`Missing additional translations: ${key}`);
  const missing=[];
  for(const file of fs.readdirSync(path.join(root,'extension')).filter(f=>f.endsWith('.js') && !f.startsWith('i18n'))) {
    for(const match of read('extension/'+file).matchAll(/__xdL\.[tf]\(("(?:[^"\\]|\\.)*")/g)) {
      const key=JSON.parse(match[1]); if(!(key in extension)) missing.push(`${file}: ${key}`);
    }
  }
  const {files,production,literals}=require('../scripts/localization-source.cjs');
  for(const file of files(path.join(root,'src')).filter(f=>f.endsWith('.rs')&&!/tests\.rs$/.test(f))) {
    const source=production(fs.readFileSync(file,'utf8'));
    for(const literal of literals(source)) {
      if(/(?:\btr\(\s*|\btf!\(\s*)$/.test(source.slice(Math.max(0,literal.start-30),literal.start)) && !(literal.value in desktop)) {
        missing.push(`${path.relative(root,file)}: ${literal.value}`);
      }
    }
  }
  assert.deepEqual(missing,[]);
});

async function runtime(uiLanguage='en-US',saved='system') {
  const listeners=[],writes=[];
  const context=vm.createContext({console,navigator:{language:uiLanguage},Event:class{constructor(type){this.type=type;}},
    dispatchEvent:event=>writes.push(event.type),
    chrome:{i18n:{getUILanguage:()=>uiLanguage},storage:{
      local:{get:async()=>({displayLanguage:saved}),set:async v=>writes.push(v)},
      onChanged:{addListener:fn=>listeners.push(fn)}
    }}});
  for(const file of ['i18n-catalog.js','i18n.js']) vm.runInContext(read('extension/'+file),context);
  await context.IDMI18n.ready;
  return {context,locale:context.IDMI18n,listeners,writes};
}

test('extension language override persists, follows browser scripts/regions and falls back safely',async()=>{
  for(const [browser,expected] of [['zh-Hans-HK','zh-CN'],['zh-HK','zh-TW'],['zh-Hant-CN','zh-TW'],['ms-MY','ms'],['ja-JP','ja'],['ko-KR','ko'],['vi-VN','vi'],['th-TH','th'],['pl-PL','pl'],['ru-RU','ru'],['uk-UA','uk'],['ar-SA','ar'],['ar-EG','ar'],['de-DE','de'],['de-AT','de'],['de_CH','de-CH'],['de-Latn-CH','de-CH'],['fr-FR','fr'],['fr-CH','fr'],['sv-SE','sv'],['nb-NO','no'],['no-NO','no'],['it-IT','it'],['it_CH','it'],['rm-CH','rm'],['rm_Latn_CH','rm'],['nn-NO','en'],['es-ES','en']]) {
    const {locale}=await runtime(browser); assert.equal(locale.language,expected);
  }
  for(const lang of languages) {
    const {locale,writes,listeners}=await runtime('zz-ZZ',lang);
    assert.equal(locale.language,lang);
    const index=languages.indexOf(lang);
    const expected=index===0?'Download video':index<4?extension['Download video'][index-1]:additional['Download video'][index-4];
    assert.equal(locale.t('Download video'),expected);
    const userValue='<script>Settings</script> {0} 你好 & a.zip';
    assert.equal(locale.t(userValue),userValue);
    assert(locale.f('Found {0} {1}. Review them in the new tab.',12,userValue).includes(userValue));
    await locale.setLanguage('zh-TW'); assert.equal(writes[0].displayLanguage,'zh-TW');
    await assert.rejects(locale.setLanguage('../en'),/Invalid/);
    listeners[0]({displayLanguage:{newValue:'ms'}},'local'); assert.equal(locale.language,'ms');
    listeners[0]({displayLanguage:{newValue:'en'}},'session'); assert.equal(locale.language,'ms');
    listeners[0]({displayLanguage:{}},'local'); assert.equal(locale.language,'en');
  }
});

test('generated extension metadata is current and every browser loads localization before consumers',()=>{
  const result=spawnSync(process.execPath,[path.join(root,'scripts/Build-Locales.cjs'),'--check'],{encoding:'utf8'});
  assert.equal(result.status,0,result.stderr);
  const english=JSON.parse(read('extension/_locales/en/messages.json'));
  for(const lang of languages) {
    const messages=JSON.parse(read(`extension/_locales/${lang.replace('-','_')}/messages.json`));
    assert.deepEqual(Object.keys(messages),Object.keys(english));
    assert(messages.extensionDescription.message.length>10);
  }
  for(const {code,extensionAliases=[]} of registry) for(const alias of extensionAliases) {
    assert.equal(read(`extension/_locales/${alias}/messages.json`),read(`extension/_locales/${code.replace('-','_')}/messages.json`));
  }
  for(const variant of ['chromium','firefox']) {
    const manifest=JSON.parse(read(`extension/manifest.${variant}.json`));
    assert.equal(manifest.default_locale,'en'); assert.equal(manifest.name,'__MSG_extensionName__');
  }
  assert(read('extension/background.js').indexOf('importScripts(')<read('extension/background.js').indexOf('const __xdL'));
  assert.deepEqual(JSON.parse(read('extension/manifest.firefox.json')).background.scripts.slice(0,2),['i18n-catalog.js','i18n.js']);
  for(const file of ['popup','login','links']) {
    const html=read(`extension/${file}.html`);
    assert(html.indexOf('src="i18n.js"')<html.indexOf(`src="${file}.js"`),file);
  }
  for(const file of ['popup','login','links','video-button']) {
    assert.match(read(`extension/${file}.js`), /\(async \(\) => \{\s*"use strict";\s*await globalThis\.IDMI18n\?\.ready;/, `${file} must retain strict mode before asynchronous initialization`);
  }
  require('../extension/core.js');require('../extension/page-links.js');
  assert(!globalThis.IDMPageLinks.collect.toString().includes('__xdL'),'Serialized page collector must not depend on extension globals');
});

test('translated guides preserve stable topic order/count and executable code examples',()=>{
  const english=read('docs/user-guide.md').split(/\r?\n## /).slice(1);
  assert.equal(english.length,12);
  for(const lang of languages.slice(1)) {
    const topics=read(`docs/locales/${lang}/user-guide.md`).split(/\r?\n## /).slice(1);
    assert.equal(topics.length,english.length);
    assert(topics[6].includes('xd.queues()')); assert(topics[6].includes('event.downloads'));
    assert(topics[7].includes('.\\de-cli.exe --help'));
    assert(topics.every(topic=>topic.includes('### ')));
  }
});

function ini(text) {
  const entries={};let section='';
  for(const line of text.replace(/^\ufeff/,'').split(/\r?\n/)) {
    if(line.startsWith('[')){section=line;continue;}
    const m=/^(\w+)=(.*)$/.exec(line);if(m)entries[section+':'+m[1]]=m[2];
  }
  return entries;
}
test('installer translations cover compiler messages and preserve substitution tokens',()=>{
  const compiler=path.join(root,'artifacts/build-tools/InnoSetup/Default.isl');
  // The compiler is an existing local release prerequisite, never downloaded by this test.
  assert(fs.existsSync(compiler),'Install the reviewed Inno Setup build tool first');
  const english=ini(fs.readFileSync(compiler,'utf8'));
  for(const file of registry.slice(1).map(language=>language.installer)) {
    const translated=ini(read('installer/languages/'+file));
    for(const [key,value] of Object.entries(english).filter(([k,v])=>k.startsWith('[Messages]') && v)) {
      assert(translated[key],`${file} missing ${key}`);
      const slots=s=>[...s.matchAll(/%[1-9n]/g)].map(m=>m[0]).filter(s=>s!=='%n').sort();
      assert.deepEqual(slots(translated[key]),slots(value),`${file}: ${key}`);
    }
  }
  for(const lang of languages) assert.equal(JSON.parse(read(`installer/languages/${lang}.json`)),lang);
});
