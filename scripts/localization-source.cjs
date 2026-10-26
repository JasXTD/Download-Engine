// Token-aware source inventory used by the localization maintenance tools.
const fs = require('node:fs');
const path = require('node:path');
function literals(source) {
  const out = [];
  const pattern = /\/\*[\s\S]*?\*\/|\/\/[^\n]*|r(#+)?"[\s\S]*?"\1|"(?:\\[\s\S]|[^"\\])*"/g;
  for (const m of source.matchAll(pattern)) {
    if (m[0].startsWith('/')) continue;
    const raw = m[0];
    let value;
    if (raw.startsWith('r')) { const q=raw.indexOf('"'); value=raw.slice(q+1,-q); }
    else {
      try { value=JSON.parse(raw.replace(/\\u\{([0-9a-f]+)\}/gi,(_,h)=>String.fromCodePoint(parseInt(h,16))).replace(/\\\r?\n\s*/g,'')); }
      catch { continue; }
    }
    out.push({start:m.index,end:m.index+raw.length,raw,value});
  }
  return out;
}
function files(directory) { return fs.readdirSync(directory,{withFileTypes:true}).flatMap(d=>d.isDirectory()?files(path.join(directory,d.name)):[path.join(directory,d.name)]); }
function production(source) { return source.split(/#\[cfg\(test\)\]\s*(?:mod tests|mod interaction_tests)\s*\{/)[0]; }
module.exports={literals,files,production};
if (require.main===module) {
 const root=path.resolve(__dirname,'..');
 const map=new Map();
 for(const file of [path.join(root,'src/desktop.rs'),...files(path.join(root,'src/desktop')).filter(f=>f.endsWith('.rs')&&!/tests\.rs$/.test(f))]) {
   const source=production(fs.readFileSync(file,'utf8'));
   for(const l of literals(source)) {
     if(!/^[A-Z][A-Za-z ]|^[A-Z]+$|^\{.*\}.*[A-Za-z]|^[a-z]+ [a-z]/.test(l.value) || /https?:|SELECT |UPDATE |INSERT |DELETE |CREATE |\.rs$|IDM_EX_|^[A-Za-z_]+\.[a-z]+$|^[A-Z_]+$/.test(l.value)&&!['APPEARANCE','DOWNLOADS','URL','SHA-256'].includes(l.value)) continue;
     if(/(?:expect|include_str|env)!?\(\s*$/.test(source.slice(Math.max(0,l.start-35),l.start))) continue;
     const item=map.get(l.value)||{key:l.value,files:[]};
     if(!item.files.includes(path.relative(root,file))) item.files.push(path.relative(root,file));
     map.set(l.value,item);
   }
 }
 const output=[...map.values()];
 fs.writeFileSync(path.join(root,'artifacts/localization-inventory.json'),JSON.stringify(output,null,2));
 console.log(JSON.stringify({count:output.length,characters:output.reduce((n,x)=>n+x.key.length,0)}));
}
