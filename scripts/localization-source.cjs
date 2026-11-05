// Token-aware source inventory used by the localization maintenance tools.
const fs = require('node:fs');
const path = require('node:path');
// Comments, raw strings, strings and char literals. A char literal such as '"', b'"' or '\''
// is consumed whole, so its quote cannot open a string that runs to the next quote in the
// file. A lifetime or loop label ('a, 'static) has no closing quote after one character and
// stays code.
const TOKENS = /\/\*[\s\S]*?\*\/|\/\/[^\n]*|r(#+)?"[\s\S]*?"\1|"(?:\\[\s\S]|[^"\\])*"|'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n\r\t])'/gu;
function literals(source) {
  const out = [];
  for (const m of source.matchAll(TOKENS)) {
    if (m[0].startsWith('/') || m[0].startsWith("'")) continue;
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
// String literals that tr( or tf!( translate. One is the first argument: rustfmt may wrap the
// call and indent it on a later line, so any whitespace may come before it. The others are the
// values a tr(if …) or tr(match …) argument can choose, such as both strings in
// tr(if cfg!(windows) { "a" } else { "b" }): a literal that is a whole block or match-arm body.
// Conditions, match patterns and arguments of other calls inside the branches are not labels.
function labels(source) {
  // Comments and literals blanked, so brackets and words inside them don't count.
  const code = source.replace(TOKENS, token => token.replace(/[^\n]/g, ' '));
  const branches = [];
  for (const m of code.matchAll(/\btr\(\s*(?:if|match)\b/g)) {
    let depth = 0, end = m.index + 2;
    for (; end < code.length; end++) {
      if (code[end] === '(') depth++;
      else if (code[end] === ')' && --depth === 0) break;
    }
    branches.push([m.index, end]);
  }
  return literals(source).filter(literal => {
    let start = literal.start, end = literal.end;
    while (start > 0 && /\s/.test(code[start - 1])) start--;
    while (end < code.length && /\s/.test(code[end])) end++;
    if (/\b(?:tr|tf!)\($/.test(code.slice(Math.max(0, start - 5), start))) return true;
    return branches.some(([from, to]) => from < literal.start && literal.end <= to)
      && /(?:\{|=>)$/.test(code.slice(Math.max(0, start - 2), start)) && /[},]/.test(code[end] ?? '');
  });
}
function files(directory) { return fs.readdirSync(directory,{withFileTypes:true}).flatMap(d=>d.isDirectory()?files(path.join(directory,d.name)):[path.join(directory,d.name)]); }
function production(source) { return source.split(/#\[cfg\(test\)\]\s*(?:mod tests|mod interaction_tests)\s*\{/)[0]; }
module.exports={literals,labels,files,production};
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
