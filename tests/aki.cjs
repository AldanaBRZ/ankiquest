const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), vm = require('node:vm');
function fixture(language = 'en') {
  const catalog = JSON.parse(fs.readFileSync(path.join(__dirname, '../static/translations-es.json'), 'utf8'));
  const context = {document:{addEventListener(){}}, location:{pathname:'/week'}, AnkiQuestI18n:{t:value=>language==='es' ? catalog[value] || value : value}};
  context.window = context;
  vm.runInNewContext(fs.readFileSync(path.join(__dirname,'../static/aki.js'),'utf8'), context);
  return context.AnkiQuestAki;
}
test('a stored freeze alone never promises protection',()=>{
  const aki=fixture();
  assert.equal(aki.mood({freezes:3,streak_state:'pending',today:{reviews:0}}),'review');
  assert.equal(aki.mood({streak_state:'protected',today:{reviews:0}}),'freeze');
  assert.equal(aki.mood({streak_state:'protected',today:{reviews:1}}),'review');
});
test('celebration requires a nonempty set of confirmed completed quests',()=>{
  const aki=fixture();
  assert.equal(aki.mood({quests:[]}),'review');
  assert.equal(aki.mood({quests:[{done:false}]}),'review');
  assert.equal(aki.mood({quests:[{done:true},{done:false}]}),'review');
  assert.equal(aki.mood({quests:[{done:true}]}),'celebrate');
  assert.equal(aki.mood({quests:[{done:'true'}]}),'review');
});
test('streak encouragement requires actual studying today',()=>{
  const aki=fixture();
  assert.equal(aki.mood({streak_state:'studied',today:{reviews:0}}),'review');
  assert.equal(aki.mood({streak_state:'studied',today:{reviews:1}}),'streak');
});
test('yesterday\'s cached profile never promises current protection or completion',()=>{
  const aki=fixture();
  const expired=Date.now()-1;
  assert.equal(aki.mood({day_ends_at:expired,streak_state:'protected',today:{reviews:0}}),'review');
  assert.equal(aki.mood({day_ends_at:expired,quests:[{done:true}]}),'review');
});
test('pose paths are fixed and optional classes cannot inject attributes',()=>{
  const markup=fixture().image('../config.json', '\" onerror=\"alert(1)');
  assert.match(markup,/src="\/aki\/welcome.png"/);
  assert.match(markup,/&quot;/);
  assert.doesNotMatch(markup,/ onerror="/);
  assert.match(markup,/alt="" aria-hidden="true"/);
});
test('Aki encouragement is translated before it reaches the page',()=>{
  const markup=fixture('es').profile({streak_state:'protected',today:{reviews:0}});
  assert.match(markup,/Un protector mantiene tu racha/);
  assert.match(markup,/data-aki-companion="freeze"/);
  assert.doesNotMatch(markup,/Your streak/);
});
