/* The hero: a live TernaryCode window, drawn with the app's own design system (app.js).
   Agents change state on their own; ⌘J (or J) jumps to the one that waits, ↵ answers it,
   ⇧⌘J goes back. The same loop as the app, with sample projects. Nothing leaves the page. */
(() => {
  const host = document.getElementById('demo-win');
  const coachEl = document.getElementById('coach');
  const demoEl = document.getElementById('demo');
  if (!host || !coachEl || typeof win !== 'function') return;

  const MAC = /Mac|iPhone|iPad|iPod/.test(navigator.platform || '') || /Mac OS X/.test(navigator.userAgent);
  const JUMP = MAC ? ['⌘', 'J'] : ['J'];
  const BACK = MAC ? ['⇧', '⌘', 'J'] : ['⇧', 'J'];
  const P = ['docs-site', 'ml-pipeline', 'mobile-app', 'payments-api', 'storefront', 'ternary-engine'];
  const IDLE = ['api-gateway', 'infra'];
  const AG = { 'mobile-app': 'CC', 'ternary-engine': 'CC', 'storefront': 'CC', 'payments-api': 'Cx', 'ml-pipeline': 'Cx', 'docs-site': 'SH' };
  const RANK = { wait: 0, fail: 1, done: 2 };

  let S, vt, lastHtml = '', prevQ = new Set(), prevSt = {}, coachKey = '', coachAt = 0, autoT = 0, endAt = 0;
  let active = false, inView = false, userDrove = false;

  function reset() {
    vt = 0; lastHtml = ''; prevQ = new Set(); prevSt = {}; coachKey = ''; endAt = 0; userDrove = false;
    S = {
      view: 'storefront', back: [],
      st: { 'docs-site': 'run', 'ml-pipeline': 'run', 'mobile-app': 'run', 'payments-api': 'run', 'storefront': 'run', 'ternary-engine': 'run' },
      since: {}, seen: {}, stage: { 'mobile-app': 0, 'ternary-engine': 0 }, stageAt: {},
    };
  }

  /* ------------------------------------------------------------ the story, on the demo's clock */
  const EVENTS = [
    [2600, () => ask('mobile-app')],
    [9800, () => ask('ternary-engine')],
    [13200, () => finish('payments-api')],
  ];
  function ask(p) { if (S.stage[p] === 0) { S.stage[p] = 1; S.stageAt[p] = vt; setSt(p, 'wait'); } }
  function finish(p) { setSt(p, 'done'); S.seen[p] = S.view === p; }
  function setSt(p, st) { S.st[p] = st; S.since[p] = vt; if (st === 'wait' || st === 'done') S.seen[p] = false; }

  function queue() {
    return P.filter(p => S.st[p] === 'wait' || (S.st[p] === 'done' && !S.seen[p]))
      .map(p => ({ p, st: S.st[p], since: S.since[p] ?? 0 }))
      .sort((a, b) => (RANK[a.st] - RANK[b.st]) || (a.since - b.since));
  }
  const nextItem = () => queue().find(i => i.p !== S.view);

  function goTo(p) {
    if (p === S.view) return;
    S.back.push(S.view);
    if (S.back.length > 20) S.back.shift();
    S.view = p;
    if (S.st[p] === 'done') S.seen[p] = true;
  }
  function jump() { const n = nextItem(); if (n) goTo(n.p); }
  function back() { if (!S.back.length) return; S.view = S.back.pop(); if (S.st[S.view] === 'done') S.seen[S.view] = true; }
  function answer() {
    const p = S.view;
    if (S.stage[p] !== 1) return;
    S.stage[p] = 2; S.stageAt[p] = vt; S.st[p] = 'run'; S.since[p] = vt;
  }
  function advance() {
    for (const p of ['mobile-app', 'ternary-engine'])
      if (S.stage[p] === 2 && vt - S.stageAt[p] > 2600) { S.stage[p] = 3; S.stageAt[p] = vt; finish(p); }
    for (const [t, f] of EVENTS) if (vt >= t && !f.done) { f.done = true; f(); }
  }

  /* ------------------------------------------------------------ text */
  const secs = ms => Math.max(0, Math.floor(ms / 1000));
  const age = ms => { const s = secs(ms); return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m`; };
  const clock = (base, ms) => { const s = base + secs(ms); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };
  const spin = (text, s) => `<span class="cc"><span class="spin">✻</span> ${text}…</span> <span class="d">(${s}s · esc to interrupt)</span>`;
  const bashLine = cmd => `<span class="g">⏺</span> <span class="b">Bash</span>(${cmd})`;
  const WHAT = {
    'mobile-app': { wait: 'Bash · flutter test', done: 'Finished · 14 tests pass' },
    'ternary-engine': { wait: 'Bash · cargo test --workspace', done: 'Finished · 42 tests pass' },
    'payments-api': { done: 'Finished · 2 files changed' },
  };
  const ASKS = { 'mobile-app': 'flutter test', 'ternary-engine': 'cargo test' };

  /* ------------------------------------------------------------ scenes */
  function storefront() {
    const all = T.storefrontAgent, t = vt;
    const n = Math.min(10, 7 + Math.floor(t / 1700));             // lines of the conversation shown
    const clicked2 = t > 7 * 1700 + 2600, hover2 = t > 7 * 1700 + 900;
    const doing = n < 8 ? 'Opening localhost:5173' : n < 9 ? 'Clicking “Add to cart” on Ridge 28 Daypack'
      : clicked2 ? 'Reading the cart badge' : 'Clicking “Add to cart” on Saddle UL Tent';
    const sp = n < 8 ? 'Opening the page' : n < 9 ? 'Adding an item to the cart' : clicked2 ? 'Checking the count goes up' : 'Adding a second item';
    const lines = [...T.ccHead(), ...all.slice(0, n), '', spin(sp, 14 + secs(t)), '', ...T.prompt(60)];
    const page = fernway({
      count: n < 9 ? 0 : clicked2 ? 2 : 1,
      added: n < 9 ? [] : clicked2 ? [0, 1] : [0],
      hl: n < 8 ? -1 : hover2 ? 1 : 0,
      cursor: n < 8 ? '' : hover2 ? 'left:452px;top:372px' : 'left:170px;top:372px',
    });
    return {
      top: topbar({ p: 'storefront', web: true, tabs: [{ al: 'CC', l: 'Claude Code', on: true }, { l: 'zsh', i: 'terminal-window' }] }),
      main: `
        <div class="stack" style="width:566px;flex:none">
          ${pane({ a: 'CC', st: 'run', stl: 'Running', meta: 'opus 4.6', hooks: true, time: clock(102, t), cls: 'focus fill' }, lines)}
          ${pane({ a: 'SH', name: 'npm run dev', st: 'run', stl: '', meta: 'node · 18m', style: 'height:300px;flex:none' }, T.vite)}
        </div>
        ${browserPanel({ agent: { a: 'CC', doing }, console: '2', page, style: 'flex:1' })}`,
    };
  }

  const FLUTTER_RUN = [
    `<span class="m">mobile-app</span> <span class="d">on</span> <span class="m"> feature/checkout</span> <span class="g">❯</span> flutter run -d macos`,
    `<span class="d">Launching lib/main.dart on macOS in debug mode...</span>`,
    `<span class="g">✓</span> Built build/macos/Build/Products/Debug/`,
    `  mobile_app.app`,
    `<span class="d">Syncing files to device macOS...</span>     <span class="d">58ms</span>`,
    '',
    `Flutter run key commands.`,
    `<span class="b">r</span> Hot reload.`,
    `<span class="b">R</span> Hot restart.`,
    `<span class="b">q</span> Quit (terminate the application).`,
    '',
    `<span class="d">flutter:</span> CheckoutPage built in 14ms`,
    `<span class="d">flutter:</span> total(<span class="y">19.99</span>, discount: <span class="y">0.15</span>) = <span class="r">16.98</span>`,
  ];
  function claudeAsk(p) {
    const stg = S.stage[p], t = vt - (S.stageAt[p] ?? 0), mob = p === 'mobile-app';
    const ask = mob ? T.flutterAsk : T.cargoAsk;
    const conv = ask.slice(0, mob ? 6 : 8);
    const cmd = mob ? 'flutter test test/checkout_test.dart' : 'cargo test --workspace';
    let lines;
    if (stg === 0) lines = [...conv.slice(0, mob ? 5 : 7), '', spin(mob ? 'Fixing the rounding' : 'Writing the tests', 31 + secs(vt))];
    else if (stg === 1) lines = ask;
    else if (stg === 2) lines = [...conv, bashLine(cmd), `  <span class="f">⎿</span>  <span class="d">${mob ? 'Running 14 tests…' : 'Compiling ternary-engine v0.3.0'}</span>`, '', spin(mob ? 'Running the checkout tests' : 'Running the tests', secs(t))];
    else lines = [...conv, bashLine(cmd),
      `  <span class="f">⎿</span>  <span class="g">${mob ? '00:04 +14: All tests passed!' : 'test result: ok. 42 passed; 0 failed'}</span>`, '',
      mob ? `<span class="g">⏺</span> The total now rounds once, after the discount:` : `<span class="g">⏺</span> <span class="b">Vec&lt;Option&lt;T&gt;&gt;</span> parses now: <span class="b">&gt;&gt;</span> closes both lists.`,
      mob ? `  19.99 − 15% = <span class="b">16.99</span>. All 14 checkout tests pass.` : `  42 tests pass, 6 of them new.`,
      '', ...T.prompt(mob ? 60 : 56)];
    const st = stg === 1 ? 'wait' : stg === 3 ? 'done' : 'run';
    const head = { a: 'CC', st, stl: { wait: 'Needs input', run: 'Running', done: 'Done' }[st], task: stg === 1 ? WHAT[p].wait : '', meta: 'opus 4.6', hooks: true,
      time: clock(mob ? 127 : 252, stg === 0 ? vt : vt - S.stageAt[p]), cls: `focus fill ${stg === 1 ? 'wait' : ''}` };
    return { lines: [...T.ccHead('opus 4.6', `~/code/${p}`), ...lines], head };
  }
  function mobile() {
    const c = claudeAsk('mobile-app'), done = S.stage['mobile-app'] === 3;
    const run = done ? [...FLUTTER_RUN, `<span class="d">Performing hot reload...</span>`, `<span class="g">Reloaded 1 of 812 libraries in 241ms.</span>`,
      `<span class="d">flutter:</span> total(<span class="y">19.99</span>, discount: <span class="y">0.15</span>) = <span class="g">16.99</span>`] : FLUTTER_RUN;
    return {
      top: topbar({ p: 'mobile-app', server: false, tabs: [{ al: 'CC', l: 'Claude Code', on: true, w: S.stage['mobile-app'] === 1 }, { l: 'flutter run', i: 'device-mobile' }] }),
      main: `${pane(c.head, c.lines)}${pane({ a: 'SH', icon: 'device-mobile', name: 'flutter run', st: 'run', stl: '', meta: 'macOS · 24m', style: 'width:470px;flex:none' }, run)}`,
    };
  }
  function engine() {
    const c = claudeAsk('ternary-engine');
    const zsh = [
      `<span class="m">ternary-engine</span> <span class="d">on</span> <span class="m"> main</span> <span class="g">❯</span> git diff --stat`,
      ` src/lexer.rs      |  6 <span class="g">++</span><span class="r">-</span>`,
      ` src/lib.rs        |  2 <span class="g">+</span><span class="r">-</span>`,
      ` src/parser.rs     | 47 <span class="g">+++++++++++</span><span class="r">---</span>`,
      ` tests/generics.rs | 64 <span class="g">++++++++++++++++</span>`,
      ` Cargo.toml        |  1 <span class="g">+</span>`,
      ` 5 files changed, 109 insertions(+), 11 deletions(-)`,
      `<span class="m">ternary-engine</span> <span class="d">on</span> <span class="m"> main</span> <span class="g">❯</span> ${CUR}`,
    ];
    return {
      top: topbar({ p: 'ternary-engine', server: false, chg: 5, tabs: [{ al: 'CC', l: 'Claude Code', on: true, w: S.stage['ternary-engine'] === 1 }, { l: 'zsh', i: 'terminal-window' }] }),
      main: `${pane(c.head, c.lines)}${pane({ a: 'SH', name: 'zsh', meta: 'zsh · 2h', style: 'width:470px;flex:none' }, zsh)}`,
    };
  }
  function payments() {
    const done = S.st['payments-api'] === 'done', all = T.codexDone;
    const lines = done ? [...all, '', ...T.prompt(56)] : [...all.slice(0, 6), '', `<span class="d">•</span> <span class="b">Working</span> <span class="d">(${12 + secs(vt)}s · esc to interrupt)</span>`];
    return {
      top: topbar({ p: 'payments-api', server: false, tabs: [{ al: 'Cx', l: 'Codex', on: true }, { l: 'zsh', i: 'terminal-window' }] }),
      main: `${pane({ a: 'Cx', st: done ? 'done' : 'run', stl: done ? 'Done' : 'Running', meta: 'gpt-5.5', hooks: true, time: clock(181, done ? 0 : vt), cls: 'focus fill' }, lines)}
        ${pane({ a: 'SH', name: 'zsh', meta: 'zsh · 1h', style: 'width:470px;flex:none' }, [
          `<span class="m">payments-api</span> <span class="d">on</span> <span class="m"> main</span> <span class="g">❯</span> go test ./...`,
          `<span class="g">ok</span>  	payments/charges	<span class="d">0.412s</span>`,
          `<span class="g">ok</span>  	payments/ledger 	<span class="d">0.236s</span>`,
          `<span class="g">ok</span>  	payments/webhook	<span class="d">0.119s</span>`,
          `<span class="m">payments-api</span> <span class="d">on</span> <span class="m"> main</span> <span class="g">❯</span> ${CUR}`])}`,
    };
  }
  function other(p) {
    if (p === 'docs-site') return { top: topbar({ p, server: 'localhost:4321', tabs: [{ l: 'npm run dev', i: 'terminal-window', on: true }] }),
      main: `${pane({ a: 'SH', name: 'npm run dev', st: 'run', stl: '', meta: 'node · 31m', cls: 'focus fill' }, T.astro)}${pane({ a: 'SH', name: 'zsh', style: 'width:470px;flex:none' }, T.idle(p))}` };
    if (p === 'ml-pipeline') return { top: topbar({ p, server: false, tabs: [{ al: 'Cx', l: 'Codex', on: true }] }),
      main: pane({ a: 'Cx', st: 'run', stl: 'Running', meta: 'gpt-5.5', hooks: true, time: clock(1514, vt), cls: 'focus fill' }, [...T.codexTrain, `  <span class="d">[${Math.min(600, 500 + Math.floor(vt / 900) * 10)}/600]</span> auc <span class="b">0.917</span>  logloss 0.226`]) };
    return { top: topbar({ p, server: false, tabs: [{ l: 'zsh', i: 'terminal-window', on: true }] }), main: pane({ a: 'SH', name: 'zsh', cls: 'focus fill' }, T.idle(p)) };
  }
  const SCENE = { 'storefront': storefront, 'mobile-app': mobile, 'ternary-engine': engine, 'payments-api': payments };

  /* ------------------------------------------------------------ the window */
  function sidebarHtml(q) {
    const pct = Math.min(97, 62 + Math.floor(vt / 1800));
    const here = q.find(i => i.p === S.view), next = q.find(i => i.p !== S.view);
    const items = q.map(i => ({ p: i.p, a: AG[i.p], st: i.st, what: WHAT[i.p][i.st], age: age(vt - i.since), here: i === here, next: i === next }));
    const rows = P.map(p => {
      const st = S.st[p], r = { n: p, sel: p === S.view };
      const unseen = q.some(i => i.p === p) && p !== S.view;
      if (p === 'docs-site') return { ...r, st: 'run', a: 'SH', stl: 'npm run dev', dots: ['r'] };
      if (p === 'ml-pipeline') return { ...r, st: 'run', a: 'Cx', stl: `Codex · ${pct}%`, dots: ['r'] };
      if (st === 'run') return { ...r, st: 'run', a: AG[p], stl: AG[p] === 'Cx' ? 'Codex' : 'Claude Code', dots: p === 'storefront' ? ['r', 'r', 'web'] : p === 'mobile-app' ? ['r', ''] : ['r'] };
      return { ...r, st, badge: unseen ? [st, 1] : null, dots: [st === 'wait' ? 'w' : 'd', ...(p === 'mobile-app' ? [''] : [])] };
    });
    const c = { run: 0, wait: 0, done: 0 };
    rows.forEach(r => c[r.st]++);
    const sum = `<span class="c-run">${lat('run')}${c.run}</span>${c.wait ? `<span class="c-wait">${IF('bell-ringing')}${c.wait}</span>` : ''}${c.done ? `<span class="c-done">${IF('check-circle')}${c.done}</span>` : ''}`;
    return sidebar({ queue: items, groups: [
      { name: 'Active', n: 6, sum, rows },
      { name: 'Idle', n: 2, rows: IDLE.map(n => ({ n, sel: n === S.view })) },
    ] });
  }
  function statusHtml(q) {
    const here = q.find(i => i.p === S.view), next = q.find(i => i.p !== S.view);
    const backK = S.back.length ? `<span class="t4" style="margin-left:4px;display:inline-flex;gap:5px;align-items:center">${kbd('⇧⌘J')} back</span>` : '';
    if (!next) return statusbar({ lead: `<span class="lead">${IF('check-circle', 'c-done')}<span class="t2">No agent is waiting for you.</span>${backK}</span>` });
    const more = q.filter(i => i !== next && i !== here).length;
    const what = next.st === 'wait'
      ? `Claude Code wants to run <span class="mono" style="font-size:10.5px;color:var(--text)">${ASKS[next.p]}</span>`
      : `${AG[next.p] === 'Cx' ? 'Codex' : 'Claude Code'} finished · ${WHAT[next.p].done.replace('Finished · ', '')}`;
    return statusbar({ lead: `<span class="lead">${next.st === 'wait' ? IF('bell-ringing', 'c-wait') : IF('check-circle', 'c-done')}${here ? '<span class="t3">Next:</span>' : ''}<b>${next.p}</b><span class="t3">·</span>${what}${more ? `<span class="t3">+${more} more</span>` : ''}${kbd('⌘J')}${backK}</span>` });
  }
  function windowHtml() {
    const q = queue();
    const sc = (SCENE[S.view] || (() => other(S.view)))();
    return { q, html: win({ theme: 'dark', sb: sidebarHtml(q), top: sc.top, main: sc.main, status: statusHtml(q) }) };
  }

  const LAT_DELAY = { 2: 0, 4: .15, 8: .3, 3: .6, 5: .75 };
  function syncLat(root) {
    const off = (performance.now() % 1200) / 1000;
    root.querySelectorAll('.lat.run').forEach(l => [...l.children].forEach((b, i) => { const d = LAT_DELAY[i + 1]; if (d !== undefined) b.style.animationDelay = `${d - off}s`; }));
  }
  function render(force) {
    const { q, html } = windowHtml();
    if (!force && html === lastHtml) return;
    lastHtml = html;
    host.innerHTML = html;
    syncLat(host);
    // what is new since the last frame: queue rows slide in, rows that changed state flash
    const qis = host.querySelectorAll('.queue .qi');
    q.forEach((it, i) => { const k = it.p + it.st; if (!prevQ.has(k) && qis[i]) qis[i].classList.add('enter'); });
    prevQ = new Set(q.map(it => it.p + it.st));
    const prs = host.querySelectorAll('.sb .pr');
    P.forEach((p, i) => { if (prevSt[p] && prevSt[p] !== S.st[p] && S.st[p] !== 'run' && prs[i]) prs[i].classList.add('flash'); });
    prevSt = { ...S.st };
    qis.forEach((el, i) => { el.dataset.p = q[i].p; });
    prs.forEach((el, i) => { el.dataset.p = [...P, ...IDLE][i]; });
    renderCoach();
  }

  /* ------------------------------------------------------------ the coach under the window */
  const keys = ks => ks.map(k => `<kbd>${k}</kbd>`).join('');
  function coachState() {
    const cur = S.view, n = nextItem();
    if (S.stage[cur] === 1) return { key: 'ask-' + cur, icon: IF('bell-ringing', 'c-wait'),
      text: `<b>Claude Code</b> asks to run <code>${ASKS[cur]}</code>.`, act: { label: 'Allow', keys: ['↵'], fn: answer } };
    if (n) return { key: 'next-' + n.p + n.st, icon: n.st === 'wait' ? IF('bell-ringing', 'c-wait') : IF('check-circle', 'c-done'),
      text: n.st === 'wait' ? `<b>${n.p}</b> needs you.` : `<b>${n.p}</b> is done. ${AG[n.p] === 'Cx' ? 'Codex' : 'Claude Code'} changed 2 files.`,
      act: { label: n.st === 'wait' ? 'Go there' : 'Take a look', keys: JUMP, fn: jump } };
    const answered = S.stage['mobile-app'] === 3 && S.stage['ternary-engine'] === 3 && S.st['payments-api'] === 'done';
    if (answered) return { key: 'end', icon: IF('check-circle', 'c-done'), end: true,
      text: `That’s the loop. <b>An agent waits, you answer, it gets back to work.</b>`, act: { label: 'Replay', icon: 'arrow-counter-clockwise', fn: replay, ghost: true } };
    if (S.stage['mobile-app'] >= 2 || S.stage['ternary-engine'] >= 2) return { key: 'working', icon: lat('run', 'lg'), text: `Back to work. <b>Nothing else needs you right now.</b>` };
    return { key: 'watch', icon: lat('run', 'lg'), text: `Five agents at work in six projects. <b>Nothing needs you yet.</b>` };
  }
  let coachS = null;
  function renderCoach() {
    const c = coachState();
    coachS = c;
    if (c.key === coachKey) return;
    coachKey = c.key; coachAt = vt;
    if (c.end) endAt = vt;
    const act = c.act ? `<button class="coach-act ${c.act.ghost ? 'ghost' : ''}" type="button" data-act>${c.act.icon ? `<i class="ic" data-i="${c.act.icon}"></i>` : ''}${c.act.label}${c.act.keys ? keys(c.act.keys) : ''}</button>` : '';
    coachEl.innerHTML = `<div class="coach-in swap">${c.icon}<span class="t">${c.text}</span>${act}</div>`;
    if (window.fillIcons) window.fillIcons(coachEl);
    syncLat(coachEl);
  }
  coachEl.addEventListener('click', e => {
    if (!e.target.closest('[data-act]') || !coachS || !coachS.act) return;
    if (coachS.act.fn !== replay) userDrove = true;
    coachS.act.fn(); render();
  });

  /* autoplay: when nobody drives, the demo answers for them after a pause */
  function autoplay() {
    if (userDrove || !coachS) return;
    if (coachS.end) { if (vt - endAt > 9000) replay(); return; }
    if (!coachS.act) return;
    const wait = coachS.key.startsWith('ask') ? 3800 : coachKey === 'next-mobile-appwait' ? 5200 : 4200;
    if (vt - coachAt > wait && !autoT) {
      const b = coachEl.querySelector('[data-act]');
      if (b) b.classList.add('press');
      autoT = setTimeout(() => { autoT = 0; if (userDrove) return; coachS.act.fn(); render(); }, 260);
    }
  }
  function replay() { clearTimeout(autoT); autoT = 0; EVENTS.forEach(e => { e[1].done = false; }); reset(); render(true); }

  /* ------------------------------------------------------------ input */
  host.addEventListener('click', e => {
    const el = e.target.closest('[data-p]');
    if (!el) return;
    userDrove = true; goTo(el.dataset.p); render();
  });
  document.addEventListener('keydown', e => {
    if (!inView || e.defaultPrevented) return;
    if (e.target.closest && e.target.closest('input, textarea, select, [contenteditable="true"]')) return;
    const k = (e.key || '').toLowerCase();
    if (k === 'j' && !e.altKey && (MAC ? !e.ctrlKey : !e.metaKey)) {
      e.preventDefault(); userDrove = true;
      e.shiftKey ? back() : jump(); render();
    } else if ((k === 'enter' || k === '1' || k === 'y') && !e.metaKey && !e.ctrlKey && !e.altKey && S.stage[S.view] === 1) {
      if (e.target.closest && e.target.closest('a, button, summary')) return;
      e.preventDefault(); userDrove = true; answer(); render();
    }
  });

  /* ------------------------------------------------------------ scale & clock */
  const scaler = host.parentElement;
  const fit = () => { const s = scaler.clientWidth / 1440; host.style.transform = `scale(${s})`; scaler.style.height = `${Math.round(900 * s)}px`; };
  new ResizeObserver(fit).observe(scaler);
  fit();

  new IntersectionObserver(es => { inView = es[0].isIntersecting && es[0].intersectionRatio > 0.3; }, { threshold: [0, .3, .6] }).observe(demoEl);
  let last = performance.now();
  setInterval(() => {
    const now = performance.now(), dt = Math.min(now - last, 500);
    last = now;
    active = inView && !document.hidden;
    if (!active) return;
    vt += dt;
    advance(); render(); autoplay();
  }, 250);
  const GL = ['·', '✢', '✳', '✶', '✻', '✽', '✻', '✶', '✳', '✢'];
  let gi = 0;
  setInterval(() => { if (!active) return; gi = (gi + 1) % GL.length; host.querySelectorAll('.spin').forEach(s => { s.textContent = GL[gi]; }); }, 130);

  reset();
  render(true);
  window.tcDemo = { replay };
})();
