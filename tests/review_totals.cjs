const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const {test}=require('node:test');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
test('Overview and Year review keep identical numeric review totals in English and Spanish',async()=>{
 const browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_CHANNEL?{channel:process.env.PLAYWRIGHT_CHANNEL}:process.platform==='win32'?{channel:'msedge'}:{})});
 try {
  const html=fs.readFileSync(path.join(__dirname,'../static/community.html'),'utf8');
  const source=[...html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)].map(m=>m[1]).find(s=>s.includes('const state ='));
  for(const locale of ['en-US','es-ES']){
   const page=await browser.newPage({locale});await page.route('**/*',r=>r.abort());
   await page.setContent(html.replace(/<script\b[^>]*>[\s\S]*?<\/script>/g,''));
   await page.addScriptTag({content:require('./site_assets.cjs').siteScript()});
   await page.addScriptTag({content:fs.readFileSync(path.join(__dirname,'../static/avatars.js'),'utf8')});
   await page.addScriptTag({content:source.slice(0,source.lastIndexOf('showView(location.hash.slice(1)||"challenges");'))});
   const totals=await page.evaluate(()=>{
    state.data={meta:{},players:[{user:'cerro',display:'Cerro',trophies:[],monthly:[],year_review:{reviews:1234,xp:9000,study_days:40,best_streak:12}}],awards:[],calendar:[],weeks:[],seasons:[],records:[],head_to_head:[]};
    renderOverview();renderYear();
    return ['view-overview','view-year'].map(id=>document.querySelectorAll('#'+id+' .kpi')[id==='view-overview'?3:1].querySelector('strong').textContent.replace(/[^0-9]/g,''));
   });
   assert.deepEqual(totals,['1234','1234'],locale);await page.close();
  }
 } finally {await browser.close();}
});
