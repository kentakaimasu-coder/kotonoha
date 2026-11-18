const { chromium } = require('/home/user/kotonoha/tests/e2e/node_modules/playwright');
const { spawn } = require('child_process');
const fs = require('fs');
const axe = fs.readFileSync('/home/user/kotonoha/tests/e2e/node_modules/axe-core/axe.min.js', 'utf8');
const S = process.argv[2];
const KOTO = process.env.KOTO_BIN || '/home/user/kotonoha/target/debug/koto';
(async () => {
  const p = spawn(KOTO, ['-C', S + '/app', 'studio', '--port', '6392'], { stdio: 'ignore', detached: true });
  const url = 'http://127.0.0.1:6392';
  for (let i = 0; i < 100; i++) { try { await fetch(url); break; } catch { await new Promise(r => setTimeout(r, 300)); } }
  // 生成アプリが起動するまで待つ
  let preview = null;
  for (let i = 0; i < 120 && !preview; i++) { try { const j = await (await fetch(url + '/api/state')).json(); if (j.previewPort && j.running) preview = j.previewPort; } catch {} if (!preview) await new Promise(r => setTimeout(r, 1000)); }
  for (let i = 0; i < 120; i++) { try { await fetch(`http://127.0.0.1:${preview}/`); break; } catch { await new Promise(r => setTimeout(r, 1000)); } }
  const b = await chromium.launch();
  const out = [];
  const audit = async (name, target, vp, login) => {
    const ctx = await b.newContext({ viewport: vp, deviceScaleFactor: 1, locale: 'ja-JP' });
    const page = await ctx.newPage();
    if (login) {
      await page.goto(`http://127.0.0.1:${preview}/login`);
      await page.fill('input[name="name"], input[type="text"]', '花子').catch(() => {});
      await page.click(`text=${login}`).catch(() => {});
      await page.waitForTimeout(500);
    }
    await page.goto(target); await page.waitForTimeout(1200);
    const file = `${S}/shots/${name}-${vp.width}.png`;
    await page.screenshot({ path: file, fullPage: true });
    const m = await page.evaluate(() => {
      const r = { overflow: document.documentElement.scrollWidth > window.innerWidth + 1, small: [], tiny: [] };
      for (const e of document.querySelectorAll('button, a, input, select, textarea, [role=button]')) {
        const b = e.getBoundingClientRect(); if (!b.width || !b.height) continue;
        if (window.innerWidth < 600 && (b.height < 40 || b.width < 40) && e.type !== 'checkbox' && e.type !== 'radio') r.small.push(`${e.tagName.toLowerCase()} "${(e.textContent || e.getAttribute('aria-label') || e.name || '').trim().slice(0, 20)}" ${Math.round(b.width)}x${Math.round(b.height)}`);
      }
      for (const e of document.querySelectorAll('body *')) {
        if (!e.childNodes.length || ![...e.childNodes].some(n => n.nodeType === 3 && n.textContent.trim())) continue;
        const fs = parseFloat(getComputedStyle(e).fontSize); if (fs < 12) r.tiny.push(`${e.tagName.toLowerCase()} ${fs}px "${e.textContent.trim().slice(0, 15)}"`);
      }
      r.small = [...new Set(r.small)]; r.tiny = [...new Set(r.tiny)];
      return r;
    });
    await page.evaluate(axe);
    const ax = await page.evaluate(async () => { const r = await window.axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'] } }); return r.violations.map(v => `${v.id}(${v.nodes.length}) ${v.help}`); });
    out.push({ name, width: vp.width, file, overflow: m.overflow, small_targets: m.small.slice(0, 15), small_count: m.small.length, tiny_text: m.tiny.slice(0, 10), axe: ax });
    await ctx.close();
  };
  const A = `http://127.0.0.1:${preview}`;
  for (const vp of [{ width: 1280, height: 860 }, { width: 390, height: 844 }]) {
    await audit('studio-home', url + '/', vp);
    await audit('app-list', A + '/', vp);
    await audit('app-detail', A + '/s/' + encodeURIComponent('イベント詳細') + '/e1', vp);
    await audit('app-login', A + '/login', vp);
    await audit('app-mine', A + '/s/' + encodeURIComponent('自分の予約'), vp, '利用者として入る');
    await audit('app-admin', A + '/s/' + encodeURIComponent('管理'), vp, '主催者として入る');
  }
  // Studio の他のタブ
  for (const vp of [{ width: 1280, height: 860 }, { width: 390, height: 844 }]) {
    const ctx = await b.newContext({ viewport: vp, locale: 'ja-JP' }); const page = await ctx.newPage();
    await page.goto(url + '/'); await page.waitForTimeout(800);
    for (const t of ['ソース', '動かす', '見た目']) { await page.click(`.tabs >> text=${t}`).catch(() => {}); await page.waitForTimeout(800); await page.screenshot({ path: `${S}/shots/studio-tab-${t}-${vp.width}.png`, fullPage: true }); }
    await page.click('#tree button:has-text("イベント詳細")').catch(() => {}); await page.waitForTimeout(800);
    await page.screenshot({ path: `${S}/shots/studio-screen-${vp.width}.png`, fullPage: true });
    await ctx.close();
  }
  await b.close();
  process.kill(-p.pid);
  fs.writeFileSync(S + '/objective.json', JSON.stringify(out, null, 1));
  for (const o of out) console.log(o.name, o.width, 'overflow=' + o.overflow, 'small=' + o.small_count, 'tiny=' + o.tiny_text.length, 'axe=' + o.axe.length, o.axe.join(' | ').slice(0, 200));
})();
