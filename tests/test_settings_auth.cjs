// Run: node --test tests/test_settings_auth.cjs
// Point PLAYWRIGHT_MODULE at an installed Playwright package if it is not on NODE_PATH.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test, before, after } = require('node:test');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

const html = fs.readFileSync(path.join(__dirname, '../static/index.html'), 'utf8');
const origin = 'http://ankiquest.test';
const savedSession = { user: 'cerro', token: 'saved-token' };
const dialogs = [
  { name: 'freezes', open: '#manage-freezes', id: '#streak-freezes', ready: '#freeze-preferences', input: '#freeze-token', unlock: '#freeze-unlock', save: 'Save preference', endpoint: 'streak-freezes' },
  { name: 'decks', open: '#manage-decks', id: '#deck-sharing', ready: '#deck-settings', input: '#deck-token', unlock: '#deck-unlock', save: 'Save preferences', endpoint: 'decks' },
];
let browser;
before(async () => {
  browser = await chromium.launch({
    headless: true,
    ...(process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : process.platform === 'win32' ? { channel: 'msedge' } : {}),
  });
});
after(async () => browser?.close());

async function fixture(t, session = savedSession, user = 'cerro') {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, colorScheme: 'dark' });
  const pendingReleases = [];
  t.after(async () => {
    pendingReleases.forEach(release => release());
    await context.close();
  });
  const page = await context.newPage();
  page.setDefaultTimeout(10000);
  page.setDefaultNavigationTimeout(15000);
  const errors = [], requests = [];
  const state = { nextFailure: null, acceptedToken: 'saved-token' };
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => requests.push({ url: request.url(), method: request.method(), auth: request.headers().authorization, body: request.postData() }));
  await page.addInitScript(session => {
    window.authStorageWrites = [];
    window.settingsFetchSignals = [];
    const fetch = window.fetch;
    window.fetch = function (input, options) {
      if (/^\/api\/(streak-freezes|decks)\//.test(String(input))) window.settingsFetchSignals.push(options.signal);
      return fetch.call(this, input, options);
    };
    const setItem = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      window.authStorageWrites.push([String(key), String(value)]);
      return setItem.call(this, key, value);
    };
    if (session) {
      window.ankiquestSession = session;
      window.dispatchEvent(new CustomEvent('ankiquest-auth'));
    }
  }, session);
  await page.route(`${origin}/**`, async route => {
    const request = route.request(), url = new URL(request.url());
    let data;
    if (url.pathname === '/') return route.fulfill({ contentType: 'text/html', body: html });
    if (url.pathname === '/api/leaderboard') data = [];
    else if (url.pathname === '/api/week') data = {};
    else if (url.pathname.startsWith('/api/profile/')) {
      const profileUser = decodeURIComponent(url.pathname.slice('/api/profile/'.length));
      data = {
        user: profileUser, display: profileUser, level: 1, xp_into_level: 0, xp_for_next: 100, xp_total: 0,
        streak: 1, freezes: 0, stored_freezes: 2, freezes_enabled: false,
        today: { reviews: 0, xp: 0 }, quests: [], heatmap: [{ date: '2026-09-22', reviews: 0, xp: 0 }],
        lifetime: { reviews: 0, hours: 0, best_streak: 1, best_combo: 0, days_active: 1 }, achievements: [],
      };
    } else if (/^\/api\/(streak-freezes|decks)\//.test(url.pathname)) {
      let forcedStatus;
      if (state.holdNext) {
        const held = state.holdNext;
        state.holdNext = null;
        held.arrive();
        await held.released;
        forcedStatus = held.status;
      }
      if (state.nextFailure) {
        const failure = state.nextFailure;
        state.nextFailure = null;
        if (failure === 'network') return route.abort('failed');
        return route.fulfill({ status: failure, body: '' });
      }
      if (forcedStatus === 401 || (forcedStatus !== 200 && request.headers().authorization !== `Bearer ${state.acceptedToken}`)) return route.fulfill({ status: 401, body: '' });
      data = url.pathname.startsWith('/api/streak-freezes/')
        ? { enabled: request.method() === 'POST' ? request.postDataJSON().enabled : false, freezes: 2, capacity: 3 }
        : { decks: [{ id: '42', name: 'Spanish', enabled: false, recipients: [] }], recipients: [], nudges: false };
    } else return route.fulfill({ status: 404, body: '' });
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify(data) });
  });
  await page.goto(`${origin}/#${user}`);
  await page.locator('#manage-freezes').waitFor();
  t.after(() => assert.deepEqual(errors, [], 'no uncaught browser errors'));
  return {
    page, state,
    holdNext(status) {
      let arrive, release;
      const arrived = new Promise(resolve => { arrive = resolve; });
      const released = new Promise(resolve => { release = resolve; });
      state.holdNext = { arrive, released, status };
      pendingReleases.push(release);
      return { arrived, release };
    },
    settingsRequests: () => requests.filter(request => /\/api\/(streak-freezes|decks)\//.test(request.url)),
    async deliver(session = savedSession) {
      await page.evaluate(value => {
        window.ankiquestSession = value;
        window.dispatchEvent(new CustomEvent('ankiquest-auth'));
      }, session);
    },
    async open(spec) { await page.locator(spec.open).click(); },
    async ready(spec) { await page.locator(spec.ready).waitFor(); },
    async close(spec) {
      await page.locator(spec.id).getByRole('button', { name: 'Close', exact: true }).click();
      await page.waitForFunction(id => !document.querySelector(id).open && document.querySelector(id).innerHTML === '', spec.id);
    },
    async unlock(spec, token = state.acceptedToken) {
      await page.locator(spec.input).fill(token);
      await page.locator(spec.unlock).getByRole('button').click();
      await page.locator(spec.ready).waitFor();
    },
    async noCredentialPersistence(...tokens) {
      const contents = await page.evaluate(() => ({
        storage: JSON.stringify([localStorage, sessionStorage, window.authStorageWrites]),
        url: location.href,
      }));
      for (const token of tokens) {
        assert.ok(!contents.storage.includes(token), 'credentials must never enter browser storage');
        assert.ok(!contents.url.includes(token), 'credentials must never enter the page URL');
        assert.ok(requests.every(request => !request.url.includes(token)), 'credentials must never enter request URLs');
        assert.ok(requests.every(request => !request.body?.includes(token)), 'credentials must only be sent in the authorization header');
      }
    },
  };
}

test('decks: an unexpected response can be retried without reopening', async t => {
  const f = await fixture(t);
  const spec = dialogs.find(dialog => dialog.name === 'decks');
  await f.page.route(`${origin}/api/decks/**`, route => route.fulfill({
    contentType: 'application/json', body: JSON.stringify({ decks: null, recipients: [], nudges: false }),
  }), { times: 1 });
  await f.open(spec);
  await f.page.locator(spec.id).getByRole('status').filter({ hasText: /sort|unexpected|try again/i }).waitFor();
  const retry = f.page.locator(spec.unlock).getByRole('button');
  assert.equal(await retry.isEnabled(), true);
  await retry.click();
  await f.ready(spec);
  assert.equal(f.settingsRequests().length, 2);
  assert.ok(f.settingsRequests().every(request => request.auth === 'Bearer saved-token'));
});

for (const spec of dialogs) {
  test(`${spec.name}: saved native credentials automatically load settings before opening`, async t => {
    const f = await fixture(t);
    await f.open(spec);
    await f.ready(spec);
    assert.equal(await f.page.locator(spec.input).count(), 0, 'an authenticated player must not need to reenter the token');
    assert.equal(f.settingsRequests().length, 1);
    assert.equal(f.settingsRequests()[0].auth, 'Bearer saved-token');
    assert.equal(f.settingsRequests()[0].method, 'GET', 'opening settings must not change preferences');
    assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'mobile settings must fit the viewport');
    if (process.env.SETTINGS_SCREENSHOT_DIR) {
      fs.mkdirSync(process.env.SETTINGS_SCREENSHOT_DIR, { recursive: true });
      await f.page.screenshot({ path: path.join(process.env.SETTINGS_SCREENSHOT_DIR, `settings-${spec.name}-mobile.png`) });
    }
    await f.close(spec);
    await f.open(spec);
    await f.ready(spec);
    await f.close(spec);
    const other = dialogs.find(candidate => candidate !== spec);
    await f.open(other);
    await f.ready(other);
    assert.equal(f.settingsRequests().length, 3);
    assert.ok(f.settingsRequests().every(request => request.auth === 'Bearer saved-token' && request.method === 'GET'));
    await f.noCredentialPersistence('saved-token');
  });

  test(`${spec.name}: native credentials arriving after opening unlock the dialog`, async t => {
    const f = await fixture(t, null);
    await f.open(spec);
    await f.page.locator(spec.input).waitFor();
    await f.deliver();
    await f.ready(spec);
    assert.equal(f.settingsRequests().length, 1);
    await f.noCredentialPersistence('saved-token');
  });

  test(`${spec.name}: duplicate auth events do not reload settings`, async t => {
    const f = await fixture(t, null);
    const held = f.holdNext();
    await f.open(spec);
    await f.deliver();
    await held.arrived;
    await f.deliver();
    await f.deliver();
    held.release();
    await f.ready(spec);
    const preference = f.page.locator(spec.name === 'freezes' ? '#freeze-enabled' : '#nudge-enabled');
    await preference.check();
    await f.deliver();
    await f.page.evaluate(() => new Promise(resolve => setTimeout(resolve, 50)));
    assert.equal(f.settingsRequests().length, 1, 'auth events while loading or already unlocked must not start another request');
    assert.equal(await preference.isChecked(), true, 'duplicate credentials must preserve unsaved preferences');
  });

  test(`${spec.name}: closing a pending dialog aborts the request and removes its auth listener`, async t => {
    const f = await fixture(t);
    const held = f.holdNext();
    await f.open(spec);
    await held.arrived;
    await f.close(spec);
    assert.equal(await f.page.evaluate(() => window.settingsFetchSignals.at(-1).aborted), true);
    held.release();
    await f.deliver();
    await f.page.evaluate(() => new Promise(resolve => setTimeout(resolve, 50)));
    assert.equal(f.settingsRequests().length, 1, 'the closed dialog must not handle late auth delivery');
    assert.equal(await f.page.locator(spec.id).innerHTML(), '');
    await f.open(spec);
    await f.ready(spec);
    assert.equal(f.settingsRequests().length, 2);
  });

  test(`${spec.name}: a public browser can enter a token once and reuse it in both dialogs`, async t => {
    const f = await fixture(t, null);
    await f.open(spec);
    assert.equal(f.settingsRequests().length, 0);
    await f.unlock(spec);
    await f.close(spec);
    await f.open(spec);
    await f.ready(spec);
    await f.close(spec);
    const other = dialogs.find(candidate => candidate !== spec);
    await f.open(other);
    await f.ready(other);
    await f.close(other);
    await f.noCredentialPersistence('saved-token');
    await f.page.reload();
    await f.open(spec);
    await f.page.locator(spec.input).waitFor();
    assert.equal(await f.page.locator(spec.input).inputValue(), '', 'a page reload must discard manually supplied credentials');
  });

  test(`${spec.name}: viewing another player never reuses the configured player's token`, async t => {
    const f = await fixture(t, savedSession, 'other');
    await f.open(spec);
    await f.page.locator(spec.input).waitFor();
    await f.deliver();
    await f.page.evaluate(() => new Promise(resolve => setTimeout(resolve, 50)));
    assert.equal(f.settingsRequests().length, 0);
    assert.equal(await f.page.locator(spec.input).inputValue(), '');
    await f.noCredentialPersistence('saved-token');
  });

  test(`${spec.name}: malformed native credentials leave a working manual fallback`, async t => {
    const f = await fixture(t, { user: 'cerro', token: 42 });
    await f.open(spec);
    await f.page.locator(spec.input).waitFor();
    assert.equal(f.settingsRequests().length, 0);
    await f.unlock(spec);
    await f.close(spec);
    await f.open(spec);
    await f.ready(spec);
    assert.equal(f.settingsRequests().length, 2);
  });

  test(`${spec.name}: removing the native account also removes its credentials for later dialogs`, async t => {
    const f = await fixture(t);
    await f.open(spec);
    await f.ready(spec);
    await f.close(spec);
    await f.deliver(null);
    await f.open(spec);
    await f.page.locator(spec.input).waitFor();
    assert.equal(await f.page.locator(spec.input).inputValue(), '');
    await f.close(spec);
    const other = dialogs.find(candidate => candidate !== spec);
    await f.open(other);
    await f.page.locator(other.input).waitFor();
    assert.equal(f.settingsRequests().length, 1, 'a cleared native credential must not survive in a second memory cache');
  });

  test(`${spec.name}: a pending native success cannot restore a removed account`, async t => {
    const f = await fixture(t);
    const held = f.holdNext(200);
    await f.open(spec);
    await held.arrived;
    await f.deliver(null);
    held.release();
    await f.page.locator(spec.input).waitFor();
    await f.close(spec);
    await f.open(spec);
    await f.page.locator(spec.input).waitFor();
    assert.equal(f.settingsRequests().length, 1);
  });

  for (const oldStatus of [200, 401]) {
    test(`${spec.name}: a pending ${oldStatus} response cannot replace newer native credentials`, async t => {
      const f = await fixture(t);
      const held = f.holdNext(oldStatus);
      await f.open(spec);
      await held.arrived;
      f.state.acceptedToken = 'replacement-token';
      await f.deliver({ user: 'cerro', token: 'replacement-token' });
      held.release();
      await f.ready(spec);
      assert.equal(f.settingsRequests().length, 2, 'the replacement credential must load settings');
      assert.equal(f.settingsRequests().at(-1).auth, 'Bearer replacement-token');
      assert.equal(await f.page.evaluate(() => window.ankiquestSession?.token), 'replacement-token');
      await f.close(spec);
      await f.deliver(null);
      await f.open(spec);
      await f.page.locator(spec.input).waitFor();
      assert.equal(f.settingsRequests().length, 2, 'neither native credential may survive removal in the manual cache');
    });
  }

  test(`${spec.name}: a native account change discards settings and ignores its pending save`, async t => {
    const f = await fixture(t);
    await f.open(spec);
    await f.ready(spec);
    const held = f.holdNext(200);
    if (spec.name === 'freezes') await f.page.locator('#freeze-enabled').check();
    else await f.page.locator('#nudge-enabled').check();
    await f.page.locator(spec.id).getByRole('button', { name: spec.save, exact: true }).click();
    await held.arrived;
    await f.deliver(null);
    held.release();
    await f.page.locator(spec.input).waitFor();
    await f.page.evaluate(() => new Promise(resolve => setTimeout(resolve, 50)));
    assert.equal(await f.page.locator(spec.ready).count(), 0, 'a completed old save must not restore the old account settings');
    assert.doesNotMatch(await f.page.locator(spec.id).getByRole('status').textContent(), /saved|protection is on/i);
    await f.close(spec);
    await f.open(spec);
    await f.page.locator(spec.input).waitFor();
    assert.equal(f.settingsRequests().length, 2);
  });

  test(`${spec.name}: an old save cannot overwrite edits made with replacement credentials`, async t => {
    const f = await fixture(t);
    await f.open(spec);
    await f.ready(spec);
    const held = f.holdNext(200);
    await f.page.locator(spec.id).getByRole('button', { name: spec.save, exact: true }).click();
    await held.arrived;
    f.state.acceptedToken = 'replacement-token';
    await f.deliver({ user: 'cerro', token: 'replacement-token' });
    await f.ready(spec);
    const preference = f.page.locator(spec.name === 'freezes' ? '#freeze-enabled' : '#nudge-enabled');
    await preference.check();
    const response = f.page.waitForResponse(response => response.request().method() === 'POST');
    held.release();
    await response;
    await f.page.evaluate(() => new Promise(resolve => setTimeout(resolve, 50)));
    assert.equal(await preference.isChecked(), true);
    assert.match(await f.page.locator(spec.id).getByRole('status').textContent(), /unsaved/i);
    assert.equal(f.settingsRequests().at(-1).auth, 'Bearer replacement-token');
  });

  test(`${spec.name}: invalid saved credentials are cleared and can be replaced without reopening`, async t => {
    const f = await fixture(t);
    f.state.acceptedToken = 'replacement-token';
    await f.open(spec);
    await f.page.locator(spec.id).getByRole('status').filter({ hasText: /not accepted|invalid|expired/i }).waitFor();
    assert.equal(await f.page.locator(spec.input).inputValue(), '');
    assert.equal(await f.page.locator(spec.unlock).getByRole('button').isEnabled(), true);
    assert.notEqual(await f.page.evaluate(() => window.ankiquestSession?.token), 'saved-token', 'a rejected native credential must be discarded');
    assert.equal(f.settingsRequests().length, 1);
    await f.unlock(spec);
    await f.close(spec);
    const other = dialogs.find(candidate => candidate !== spec);
    await f.open(other);
    await f.ready(other);
    assert.equal(f.settingsRequests().at(-1).auth, 'Bearer replacement-token');
    await f.noCredentialPersistence('saved-token', 'replacement-token');
  });

  test(`${spec.name}: expired credentials during save expose a retryable token form`, async t => {
    const f = await fixture(t);
    await f.open(spec);
    await f.ready(spec);
    f.state.acceptedToken = 'replacement-token';
    await f.page.locator(spec.id).getByRole('button', { name: spec.save, exact: true }).click();
    await f.page.locator(spec.input).waitFor();
    await f.page.locator(spec.id).getByRole('status').filter({ hasText: /no longer accepted/i }).waitFor();
    assert.equal(await f.page.locator(spec.input).inputValue(), '');
    await f.unlock(spec);
    assert.equal(f.settingsRequests().at(-1).auth, 'Bearer replacement-token');
  });

  test(`${spec.name}: failed loading keeps an enabled retry control and can recover`, async t => {
    const f = await fixture(t);
    f.state.nextFailure = 'network';
    await f.open(spec);
    await f.page.locator(spec.id).getByRole('status').filter({ hasText: /failed|could not|try again|network/i }).waitFor();
    const retry = f.page.locator(spec.id).getByRole('button', { name: /retry|try again|load my/i });
    assert.equal(await retry.isEnabled(), true);
    await retry.click();
    await f.ready(spec);
    assert.equal(f.settingsRequests().length, 2);
  });
}
