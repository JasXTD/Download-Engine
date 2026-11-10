'use strict';
// Operates only the AT-SPI bus in an already-running, disposable Wayland session.
// The Rust probe imports the app's real Unix chooser adapter. No mocked portal.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {execFile} = require('node:child_process');
const {promisify} = require('node:util');
const run = promisify(execFile);
assert(process.argv[2], 'Usage: node scripts/Test-WaylandDialogs.cjs <evidence-directory> [inspect|step|all]');
const mode=process.argv[3] || 'inspect';
assert(['inspect','step','all'].includes(mode),'Unknown driver mode');
const evidence = path.resolve(process.argv[2]);
let root, pid, address, destination;
async function linux(...args) {
  const r = await run('wsl.exe', ['-d','Ubuntu','--exec',...args], {timeout:15000,maxBuffer:2*1024*1024});
  return r.stdout.trim();
}
function refs(text) {
  return [...text.matchAll(/\('([^']+)', (?:objectpath )?'([^']+)'\)/g)].map(m=>({bus:m[1],object:m[2]}));
}
function quoted(text) {
  const match = /^\('((?:\\.|[^'])*)'/.exec(text);
  return match ? match[1].replace(/\\'/g,"'").replace(/\\\\/g,'\\') : text;
}
async function call(object, method, ...args) {
  return linux('gdbus','call','--address',address,'--dest',destination,'--object-path',object,'--method',method,...args);
}
async function init() {
  root=fs.readFileSync(path.join(evidence,'runtime-root'),'utf8').trim();
  assert.match(root,/^\/tmp\/de-wayland-dialogs\.[A-Za-z0-9]+$/);
  const candidates = (await linux('sh','-c','for p in /proc/[0-9]*/root/evidence/phase; do test -f "$p" && printf "%s\\n" "$p"; done')).split('\n');
  const found=[];
  for(const candidate of candidates) {
    if(!/^\/proc\/\d+\/root\/evidence\/phase$/.test(candidate))continue;
    const inode=await linux('stat','-Lc','%d:%i',candidate);
    if(inode === await linux('stat','-Lc','%d:%i',`${root}/evidence/phase`))found.push(candidate.split('/')[2]);
  }
  assert(found.length,'No owned test session found');
  pid=found[0];
  address=`unix:path=/proc/${pid}/root/run/user/1000/at-spi/bus`;
  const apps=refs(await linux('gdbus','call','--address',address,'--dest','org.a11y.atspi.Registry','--object-path','/org/a11y/atspi/accessible/root','--method','org.a11y.atspi.Accessible.GetChildren'));
  const matches=[];
  for(const app of apps) {
    destination=app.bus;
    const p=await call(app.object,'org.freedesktop.DBus.Properties.GetAll','org.a11y.atspi.Accessible');
    if(p.includes("'Name': <'xdg-desktop-portal-gtk'>"))matches.push(app.bus);
  }
  assert.equal(matches.length,1,'Expected one isolated GTK portal');
  destination=matches[0];
  console.log(`Connected to owned GTK chooser bus (${destination}).`);
}
async function tree() {
  let pending=[{object:'/org/a11y/atspi/accessible/root',depth:0}];
  const seen=new Set(), rows=[];
  while(pending.length) {
    const batch=pending.splice(0,5).filter(n=>!seen.has(n.object));
    for(const n of batch)seen.add(n.object);
    assert(seen.size<=350,'Unexpectedly large test UI tree');
    const found=await Promise.all(batch.map(async n=>{
      const records=(await linux('sh','-ec',
        'gdbus call --address "$1" --dest "$2" --object-path "$3" --method org.freedesktop.DBus.Properties.GetAll org.a11y.atspi.Accessible; for method in GetChildren GetRoleName GetState; do gdbus call --address "$1" --dest "$2" --object-path "$3" --method "org.a11y.atspi.Accessible.$method"; done',
        'read-accessible',address,destination,n.object)).split('\n');
      assert.equal(records.length,4,'Unexpected accessibility record format');
      const [properties,links,role,state]=records;
      const name=/'Name': <'((?:\\.|[^'])*)'>/.exec(properties)?.[1] || '';
      const bits=Number(/\[(?:uint32 )?(\d+)/.exec(state)?.[1] || 0);
      return {...n,name,role:quoted(role),state,bits,showing:!!(bits&2**25),children:refs(links).filter(r=>r.bus===destination).map(r=>r.object)};
    }));
    for(const row of found) {
      rows.push(row);
      if(row.depth<20)for(const object of row.children)pending.push({object,depth:row.depth+1});
    }
  }
  fs.writeFileSync(path.join(evidence,'accessible-tree.json'),JSON.stringify(rows,null,2));
  return rows;
}
async function actions(row) { return call(row.object,'org.a11y.atspi.Action.GetActions'); }
async function activate(row) {
  const supported=await actions(row);
  const names=[...supported.matchAll(/\('([^']+)',/g)].map(m=>m[1]);
  const index=names.findIndex(name=>/^(click|activate|press)$/i.test(name));
  assert(index>=0,`No activation action for ${row.name}: ${supported}`);
  assert.match(await call(row.object,'org.a11y.atspi.Action.DoAction',String(index)),/true/);
}
async function phase() { return linux('cat',`${root}/evidence/phase`); }
function one(rows,predicate,label) {
  const found=rows.filter(predicate);
  assert.equal(found.length,1,`Expected one ${label}; found ${found.length}`);
  return found[0];
}
async function button(rows,name) {
  const row=one(rows,r=>r.showing && r.role==='button' && r.name===name,`${name} button`);
  await activate(row);
}
async function selectFile(rows,name) {
  const table=one(rows,r=>r.showing && r.role==='table' && r.name==='Files','files table');
  let cell=one(rows,r=>r.showing && r.role==='table cell' && r.name===name,`file ${name}`);
  while(!table.children.includes(cell.object))cell=one(rows,r=>r.children.includes(cell.object),'cell parent');
  const index=table.children.indexOf(cell.object);
  const rowResult=await call(table.object,'org.a11y.atspi.Table.GetRowAtIndex',String(index));
  const rowIndex=Number(/\((-?\d+),/.exec(rowResult)?.[1]);
  assert(Number.isSafeInteger(rowIndex) && rowIndex>=0,'Invalid table row');
  assert.match(await call(cell.object,'org.a11y.atspi.Component.GrabFocus'),/true/);
  const selected=await call(table.object,'org.a11y.atspi.Table.GetSelectedRows');
  assert.match(selected,new RegExp(`\\b${rowIndex}\\b`),'The row was not selected');
}
async function changed(previous) {
  const until=Date.now()+12000;
  while(Date.now()<until) {
    const current=await phase();
    if(current!==previous)return current;
    await new Promise(r=>setTimeout(r,150));
  }
  throw new Error(`Native chooser did not return for ${previous}`);
}
async function step(rows,current) {
  if(current==='folder-select') {
    await selectFile(rows,'選択 folder');
    await button(rows,'Select');
  } else if(current==='open-select') {
    assert(!rows.some(r=>r.showing && r.name==='excluded.txt'),'JSON filter exposed the excluded text fixture');
    await selectFile(rows,'résumé 日本語.json');
    await button(rows,'Select');
  } else if(['folder-cancel','open-cancel','save-cancel'].includes(current)) {
    await button(rows,'Cancel');
  } else if(['save-new','extension-collision'].includes(current)) {
    await button(rows,'Save');
  } else if(['overwrite-decline','overwrite-confirm'].includes(current)) {
    await button(rows,'Save');
    const confirmation=await tree();
    const replace=one(confirmation,r=>r.showing && r.role==='button' && r.name==='Replace','overwrite confirmation');
    fs.writeFileSync(path.join(evidence,`${current}-confirmation.json`),JSON.stringify(confirmation,null,2));
    if(current==='overwrite-confirm') {
      await activate(replace);
    } else {
      let ancestor=replace;
      while(ancestor.role!=='alert' && ancestor.role!=='dialog')ancestor=one(confirmation,r=>r.children.includes(ancestor.object),'confirmation parent');
      const descendants=new Set([ancestor.object]);
      for(let count=0;count<20;count++)for(const row of confirmation)if(descendants.has(row.object))for(const child of row.children)descendants.add(child);
      await button(confirmation.filter(r=>descendants.has(r.object)),'Cancel');
      await button(await tree(),'Cancel');
    }
  } else throw new Error(`Unsupported phase ${current}`);
  const next=await changed(current);
  console.log(`Passed ${current}; next=${next}`);
}
async function main() {
  await init();
  for(let count=0;count<10;count++) {
    const current=await phase();
    if(current==='done') { console.log('All native dialog scenarios passed.'); return; }
    const rows=await tree();
    console.log(`Phase: ${current}`);
    if(mode!=='inspect')await step(rows,current);
    else for(const r of rows.filter(r=>r.showing || r.depth<2))console.log(`${r.object.split('/').pop()} ${r.role} ${JSON.stringify(r.name)} ${r.state}`);
    if(mode!=='all')return;
  }
  throw new Error('Too many test phases');
}
main().catch(e=>{console.error(e.stack);process.exitCode=1;});
