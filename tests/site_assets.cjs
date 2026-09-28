const fs=require('node:fs'),path=require('node:path');
const root=path.join(__dirname,'../static');
exports.siteScript=()=> [
  ['Spanish','es'],['French','fr'],['German','de'],['Portuguese','pt'],
].map(([name,code])=>`window.AnkiQuest${name}=${fs.readFileSync(path.join(root,`translations-${code}.json`),'utf8')};`).join('\n')+'\n'+fs.readFileSync(path.join(root,'i18n.js'),'utf8')+'\n'+fs.readFileSync(path.join(root,'site.js'),'utf8');
