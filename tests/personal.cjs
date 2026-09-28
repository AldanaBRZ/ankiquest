// Browser checks for the personal pages using the production assets and scoped API fixtures.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '..');
const asset = file => fs.readFileSync(path.join(root,'static',file),'utf8');
const siteScript = `window.AnkiQuestSpanish=${asset('translations-es.json')};\n${asset('i18n.js')}\n${asset('site.js')}`;
const now = Date.now();
const token = 'test-owner-token';
const initial = () => ({
  member:true,
  profile:{user:'alice',display:'Alice <img src=x onerror=alert(1)>',today:{reviews:12,xp:150},streak:5,level:3,quests:[{title:'Review 10 cards',progress:10,target:10,reward:50,done:true},{title:'Review 20 cards',progress:12,target:20,reward:50,done:false}]},
  study:{year:2026,years:[2026,2025],days:[{date:'2026-09-24',reviews:1,time_ms:30000,xp:12,new_cards:0,streak:1,frozen:false},{date:'2026-09-25',reviews:3,time_ms:120000,xp:41,new_cards:1,streak:2,frozen:false},{date:'2026-09-26',reviews:0,time_ms:0,xp:0,new_cards:0,streak:2,frozen:true}],last_review_at:now-60000,last_received_at:now,latest_session:{started_at:now-180000,ended_at:now-60000,reviews:2,time_ms:90000,new_cards:1}},
  challenges:{challenges:[{id:7,title:'Seven days together',status:'active',members:[{user:'alice',status:'invited'}]}]},
  activity:{items:[],unread_count:2},
  reminders:{gentle_daily:false,urgent_streak:false,freeze_used:false,freeze_refill:false,milestone:false,weekly_closing:false,weekly_recap:false,reminder_hour:20,quiet_start:22,quiet_end:9,daily_limit:2},
  freezes:{enabled:false,freezes:1,capacity:3},
  nudges:{receiving:true,automatic_receiving:false,friends:[{user:'bob',display:'Bob',muted_by_me:false,enabled:true,sent_today:false}]},
  subscriptions:{enabled:true,unsubscribed_senders:[],sharing_senders:['bob'],senders:[{user:'bob',display:'Bob'}]},
  decks:{decks:[{id:'1',name:'Spanish',enabled:false,recipients:[]}],recipients:[{user:'bob',display:'Bob'}],nudges:false,celebrations:true},
  writes:[],
});
async function fixture(browser, options={}) {
  const data = initial();
  if(options.member === false)data.member=false;
  const context = await browser.newContext({locale:options.locale||'en-US',viewport:{width:options.width||390,height:850},colorScheme:options.theme||'light'});
  await context.route('**/*', async route => {
    const url=new URL(route.request().url()), pathname=url.pathname, method=route.request().method();
    const files={'/today':'personal.html','/history':'personal.html','/settings':'personal.html','/personal.js':'personal.js','/personal.css':'personal.css','/site.css':'site.css','/avatars.css':'avatars.css','/avatars.js':'avatars.js'};
    if(pathname==='/site.js')return route.fulfill({contentType:'text/javascript',body:siteScript});
    if(files[pathname])return route.fulfill({contentType:pathname.endsWith('.js')?'text/javascript':pathname.endsWith('.css')?'text/css':'text/html',body:asset(files[pathname])});
    if(pathname==='/auth/status')return route.fulfill({json:{private_site:true,authenticated:true,member:data.member?{user:'alice'}:null}});
    if(pathname==='/auth/session'){data.member=true;return route.fulfill({json:{}});}
    if(pathname==='/auth/logout'){data.member=false;return route.fulfill({json:{}});}
    if(pathname==='/api/profile/alice')return route.fulfill({json:data.profile});
    if(pathname==='/api/study/alice')return route.fulfill({json:url.searchParams.get('year')==='2025'?{...data.study,year:2025,days:[{date:'2025-06-20',reviews:4,time_ms:120000,xp:40,new_cards:1,streak:1,frozen:false}]}:data.study});
    if(pathname==='/api/activity/alice')return route.fulfill({json:data.activity});
    if(pathname==='/api/community/challenges/alice')return route.fulfill({json:data.challenges});
    const settings={'/api/community/reminders/alice':'reminders','/api/streak-freezes/alice':'freezes','/api/friend-nudges/alice':'nudges','/api/deck-subscriptions/alice':'subscriptions','/api/decks/alice':'decks'};
    if(settings[pathname]) {
      const key=settings[pathname];
      if(method==='POST'){const body=route.request().postDataJSON();data.writes.push({pathname,body});data[key]={...data[key],...body};}
      return route.fulfill({json:data[key]});
    }
    if(pathname.startsWith('/api/friend-nudges/alice/')) {
      const body=route.request().postDataJSON();data.writes.push({pathname,body});
      if(pathname.endsWith('/automatic'))data.nudges.automatic_receiving=body.enabled;
      else if(pathname.endsWith('/receiving'))data.nudges.receiving=body.enabled;
      else data.nudges.friends[0].muted_by_me=!body.enabled;
      return route.fulfill({status:204});
    }
    if(pathname==='/manifest.webmanifest')return route.fulfill({json:{name:'AnkiQuest'}});
    if(pathname==='/icon.svg')return route.fulfill({contentType:'image/svg+xml',body:'<svg xmlns="http://www.w3.org/2000/svg"/>'});
    return route.fulfill({status:404,body:pathname});
  });
  const page=await context.newPage(), errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  return {page,context,data,errors};
}
async function noOverflow(page,label){const size=await page.evaluate(()=>({width:innerWidth,scroll:document.documentElement.scrollWidth}));assert(size.scroll<=size.width,`${label}: ${JSON.stringify(size)}`);}
(async()=>{
  const browser=await chromium.launch({headless:true,channel:process.env.PLAYWRIGHT_CHANNEL||'msedge'});
  const screenshots=process.env.ANKIQUEST_PERSONAL_QA_DIR;
  if(screenshots)fs.mkdirSync(screenshots,{recursive:true});
  let checks=0;
  try {
    for(const width of [320,390,1440])for(const theme of ['light','dark']) {
      const f=await fixture(browser,{width,theme});
      await f.page.goto('http://ankiquest.test/today');
      await f.page.getByRole('heading',{name:/Alice/}).waitFor();
      assert.equal(await f.page.locator('img[onerror]').count(),0);
      assert.equal(await f.page.locator('.personal-quest').count(),2);
      assert.equal(await f.page.locator('.personal-summary strong').first().textContent(),'2');
      assert.match(await f.page.locator('.personal-stamp').textContent(),/Last upload received/);
      assert.match(await f.page.locator('a[href="/community#challenge-7"]').getAttribute('href'),/challenge-7/);
      assert.match(await f.page.locator('a[href="/community#activity"]').textContent(),/2 new updates/);
      await noOverflow(f.page,`today ${width} ${theme}`);
      if(screenshots && width===390)await f.page.screenshot({path:path.join(screenshots,`today-${theme}.png`),fullPage:true});
      await f.page.goto('http://ankiquest.test/history');
      await f.page.locator('[data-day="2026-09-25"]').click();
      assert.match(await f.page.locator('#day-detail').textContent(),/41/);
      assert.equal(await f.page.locator('[data-day="2026-09-25"]').getAttribute('aria-pressed'),'true');
      await f.page.locator('#history-year').selectOption('2025');
      await f.page.locator('[data-day="2025-06-20"]').waitFor();
      await noOverflow(f.page,`history ${width} ${theme}`);
      if(screenshots && width===390)await f.page.screenshot({path:path.join(screenshots,`history-${theme}.png`),fullPage:true});
      assert.deepEqual(f.errors,[]);
      await f.context.close();checks++;
    }
    const f=await fixture(browser);
    await f.page.goto('http://ankiquest.test/settings');
    await f.page.locator('#reminders-form').waitFor();
    if(screenshots)await f.page.screenshot({path:path.join(screenshots,'settings-light.png'),fullPage:true});
    await f.page.locator('#freeze-toggle').check();
    await f.page.waitForFunction(()=>document.querySelector('#freeze-status')?.textContent.includes('Saved'));
    assert.deepEqual(f.data.writes.at(-1),{pathname:'/api/streak-freezes/alice',body:{enabled:true}});
    await f.page.locator('#reminders-form [name="gentle_daily"]').check();
    await f.page.locator('#reminders-form button[type="submit"]').click();
    await f.page.waitForFunction(()=>document.querySelector('#reminders-form .status')?.textContent.includes('Saved'));
    assert.equal(f.data.writes.at(-1).body.gentle_daily,true);
    await f.page.locator('[data-nudge-field="automatic"]').check();
    await f.page.waitForFunction(()=>document.querySelector('#nudge-status')?.textContent.includes('Saved'));
    assert.equal(f.data.writes.at(-1).pathname,'/api/friend-nudges/alice/automatic');
    await f.page.locator('#subscriptions-form [data-unsubscribe="bob"]').check();
    await f.page.locator('#subscriptions-form button[type="submit"]').click();
    await f.page.waitForFunction(()=>document.querySelector('#subscriptions-form .status')?.textContent.includes('Saved'));
    assert.deepEqual(f.data.writes.at(-1).body.unsubscribed_senders,['bob']);
    await f.page.locator('#decks-form summary').click();
    await f.page.locator('[data-deck-enabled]').check();
    await f.page.locator('[data-deck-recipient="bob"]').check();
    await f.page.locator('#decks-form button[type="submit"]').click();
    await f.page.waitForFunction(()=>document.querySelector('#decks-form .status')?.textContent.includes('Saved'));
    assert.deepEqual(f.data.writes.at(-1).body.decks,[{id:'1',enabled:true,recipients:['bob']}]);
    assert.deepEqual(f.errors,[]);checks++;await f.context.close();
    const gate=await fixture(browser,{member:false});
    await gate.page.goto('http://ankiquest.test/today');
    await gate.page.locator('#personal-gate:visible').waitFor();
    await gate.page.locator('[name="user"]').fill('alice');
    await gate.page.locator('[name="token"]').fill(token);
    await gate.page.locator('#personal-connect button').click();
    await gate.page.getByRole('heading',{name:/Alice/}).waitFor();
    assert.equal(await gate.page.locator('[name="token"]').inputValue(),'');
    assert.deepEqual(gate.errors,[]);checks++;await gate.context.close();
    const spanish=await fixture(browser,{locale:'es-ES'});
    await spanish.page.goto('http://ankiquest.test/settings');
    await spanish.page.getByRole('heading',{name:'Tus ajustes'}).waitFor();
    assert.equal(await spanish.page.locator('[data-personal-tab=settings]').textContent(),'Ajustes');
    await spanish.page.goto('http://ankiquest.test/today');
    await spanish.page.getByRole('heading',{name:'Última sesión sincronizada'}).waitFor();
    assert.deepEqual(spanish.errors,[]);checks++;await spanish.context.close();
  } finally {await browser.close();}
  console.log(`PASS: ${checks} personal-page responsive, privacy, history and settings browser scenarios.`);
})().catch(error=>{console.error(error);process.exitCode=1;});
