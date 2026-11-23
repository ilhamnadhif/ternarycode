/* The hero: a live TernaryCode window, drawn with the app's own design system (app.js).
   Agents change state on their own; ⌘J (or J) jumps to the one that waits, ↵ approves it,
   ⇧⌘J goes back. The same loop as the app, with sample projects. Nothing leaves the page. */
(() => {
  const host = document.getElementById('demo-win');
  const coachEl = document.getElementById('coach');
  const demoEl = document.getElementById('demo');
  if (!host || !coachEl || typeof win !== 'function') return;

  const MAC = /Mac|iPhone|iPad|iPod/.test(navigator.platform || '') || /Mac OS X/.test(navigator.userAgent);
  const JUMP = MAC ? ['⌘', 'J'] : ['J'];
  const P = PROJECTS.map(p => p.n);
  const RANK = { inp: 0, fail: 1, done: 2 };
  // The two agents that stop to ask, what they ask, and who they are.
  const ASK = {
    'storefront': { a: 'CC', who: 'Claude Code', cmd: 'npm test -- auth', body: 'Bash · npm test -- auth' },
    'ternarycode': { a: 'Cx', who: 'Codex', cmd: 'cargo test --workspace', body: 'cargo test --workspace' },
  };

  let S, vt, lastHtml = '', prevSt = {}, coachKey = '', coachAt = 0, autoT = 0, endAt = 0;
  let active = false, inView = false, userDrove = false;

  function reset() {
    vt = 0; lastHtml = ''; prevSt = {}; coachKey = ''; endAt = 0; userDrove = false;
    S = {
      view: 'storefront', back: [],
      st: { 'storefront': 'run', 'ternarycode': 'run', 'api-gateway': 'run', 'mobile-app': 'fail', 'docs-site': null, 'infra': null },
      since: { 'mobile-app': -240000 }, seen: { 'mobile-app': true }, stage: { 'storefront': 0, 'ternarycode': 0 }, stageAt: {},
    };
  }

  /* ------------------------------------------------------------ the story, on the demo's clock */
  const EVENTS = [
    [2600, () => ask('storefront')],
    [9800, () => ask('ternarycode')],
    [13200, () => finish('api-gateway')],
  ];
  function ask(p) { if (S.stage[p] === 0) { S.stage[p] = 1; S.stageAt[p] = vt; setSt(p, 'inp'); } }
  function finish(p) { setSt(p, 'done'); S.seen[p] = S.view === p; }
  function setSt(p, st) { S.st[p] = st; S.since[p] = vt; if (st === 'inp' || st === 'done') S.seen[p] = false; }

  // The Inbox: what waits for you, in the order ⌘J visits it.
  function queue() {
    return P.filter(p => S.st[p] === 'inp' || ((S.st[p] === 'done' || S.st[p] === 'fail') && !S.seen[p]))
      .map(p => ({ p, st: S.st[p], since: S.since[p] ?? 0 }))
      .sort((a, b) => (RANK[a.st] - RANK[b.st]) || (a.since - b.since));
  }
  const nextItem = () => queue().find(i => i.p !== S.view);

  function goTo(p) {
    if (p === S.view || !P.includes(p)) return;
    S.back.push(S.view);
    if (S.back.length > 20) S.back.shift();
    S.view = p;
    if (S.st[p] === 'done' || S.st[p] === 'fail') S.seen[p] = true;
  }
  function jump() { const n = nextItem(); if (n) goTo(n.p); }
  function back() { if (!S.back.length) return; S.view = S.back.pop(); if (S.st[S.view] === 'done') S.seen[S.view] = true; }
  function answer() {
    const p = S.view;
    if (S.stage[p] !== 1) return;
    S.stage[p] = 2; S.stageAt[p] = vt; S.st[p] = 'run'; S.since[p] = vt;
  }
  function advance() {
    for (const p of Object.keys(ASK))
      if (S.stage[p] === 2 && vt - S.stageAt[p] > 2600) { S.stage[p] = 3; S.stageAt[p] = vt; finish(p); }
    for (const [t, f] of EVENTS) if (vt >= t && !f.done) { f.done = true; f(); }
  }

  /* ------------------------------------------------------------ text */
  const secs = ms => Math.max(0, Math.floor(ms / 1000));
  const clock = (base, ms) => { const s = base + secs(ms); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };
  const ago = (base, ms) => `${base + Math.floor(secs(ms) / 60)}m`;
  const ccSpin = (text, s) => `<span class="a"><span class="tspin">✻</span> ${text}…</span> <span class="d">(${s}s · esc to interrupt)</span>`;
  const cxWork = s => `<span class="d">◦ Working (${s}s • esc to interrupt)</span>`;
  const usagePct = () => 42 + Math.min(4, Math.floor(vt / 8000));
  const stOf = stg => (stg === 1 ? 'inp' : stg === 3 ? 'done' : 'run');

  /* ------------------------------------------------------------ the projects, one scene each */
  function storefront() {
    const stg = S.stage.storefront, t = vt, ts = vt - (S.stageAt.storefront ?? 0);
    const head = [
      `<span class="d">&gt; </span><span class="p">Fix the OAuth redirect loop after the callback. Users</span>`,
      `<span class="p">  bounce between /login and /auth/callback when returnTo</span>`,
      `<span class="p">  is missing.</span>`, '',
      `<span class="a">⏺ </span>I'll trace where returnTo is read and written.`, '',
      `<span class="a">⏺ </span><span class="p">Search</span><span class="d">(pattern: "returnTo", path: "src")</span>`,
      `<span class="d">  ⎿  Found 6 files</span>`,
      `<span class="a">⏺ </span><span class="p">Read</span><span class="d">(src/auth/callback.ts)</span>`,
      `<span class="d">  ⎿  Read 84 lines</span>`, '',
    ];
    const explain = [`<span class="a">⏺ </span>The callback redirects to session.returnTo even when it`, `  is empty, so the login page sends users straight back.`, ''];
    const update = [
      `<span class="a">⏺ </span><span class="p">Update</span><span class="d">(src/auth/callback.ts)</span>`,
      `<span class="d">  ⎿  Updated with 6 additions and 2 removals</span>`,
      `<span class="d">      38   </span>const session = await getSession(req)`,
      `<span class="d">      39 </span><span class="r">-  return redirect(session.returnTo)</span>`,
      `<span class="d">      39 </span><span class="g">+  const target = safeReturnTo(session.returnTo)</span>`,
      `<span class="d">      40 </span><span class="g">+  return redirect(target ?? "/account")</span>`, '',
    ];
    const bash = `<span class="a">⏺ </span><span class="p">Bash</span><span class="d">(npm test -- auth)</span>`;
    let lines;
    if (stg === 0) lines = [...head, ...(t > 900 ? explain : []), ...(t > 1800 ? update : []), ccSpin(t > 1800 ? 'Checking the fix' : 'Reading the callback', 61 + secs(t))];
    else if (stg === 1) lines = [...head, ...explain, ...update, bash, `<span class="y">  ⎿  Waiting for your approval…</span>`];
    else if (stg === 2) lines = [...head, ...explain, ...update, bash, `<span class="d">  ⎿  Running…</span>`, '', ccSpin('Running the auth tests', secs(ts))];
    else lines = [...head, ...explain, ...update, bash, `<span class="d">  ⎿  </span><span class="g">Tests: 9 passed, 9 total</span>`, '',
      `<span class="a">⏺ </span>Fixed. An empty or off-site returnTo now falls back to`, `  /account. All 9 auth tests pass.`, '', `<span class="d">&gt; </span>${CUR}`];
    const st = stOf(stg);
    const task = { run: stg === 2 ? 'Bash · npm test -- auth' : t > 1800 ? 'Editing src/auth/callback.ts' : 'Reading src/auth', inp: 'Bash · npm test -- auth', done: '2 files · +6 −2' }[st];
    const note = stg === 1 ? notice('inp', 'Claude Code wants to run a command', { body: ASK.storefront.body, sec: 'Reject', pri: 'Approve', sc: '↵', priAttr: 'data-approve' })
      : stg === 3 ? notice('done', 'Claude Code finished its turn', { body: '2 files changed · +6 −2', sec: 'Review', pri: 'Commit…' }) : '';
    const vite = [...TX.vite, ...(t > 1800 || stg > 0 ? [TX.hmr('12:41:07', '/src/auth/callback.ts'), TX.hmr('12:41:09', '/src/auth/session.ts')] : [])];
    return {
      bar: { branch: 'main', changed: 5, pr: [128, 'running'], server: 'localhost:5173' },
      tabs: [{ a: 'CC', t: 'Fix OAuth redirect loop', st, on: true }, { a: 'Cx', t: 'Add order export to CSV', st: 'run' }, { t: 'dev server' }],
      sessions: [{ a: 'CC', t: 'Fix OAuth redirect loop', meta: ago(2, vt), st, on: true }, { a: 'Cx', t: 'Add order export to CSV', meta: ago(14, vt), st: 'run' }, { a: 'SH', t: 'npm run dev', meta: ':5173' }],
      worktrees: [{ n: 'codex-export', br: 'tc/export', st: 'run' }],
      lsp: 'typescript',
      body: hsplit(
        pane({ a: 'CC', name: 'Fix OAuth redirect loop', st, task, timer: clock(132, vt), profile: `claude1 · ${usagePct()}%`, focus: true, wait: stg === 1, notice: note, lines }),
        vsplit(pane({ name: 'npm run dev', st: 'run', task: 'localhost:5173', lines: vite }),
          pane({ name: 'zsh', place: '~/code/storefront', lines: TX.gitStatus }))),
    };
  }

  function ternarycode() {
    const stg = S.stage.ternarycode, ts = vt - (S.stageAt.ternarycode ?? 0);
    const head = [
      `<span class="d">› </span><span class="p">Make the find bar count its matches, then run the tests</span>`, '',
      `<span class="c">• </span>Explored`,
      `<span class="d">  └ Read src/term/find.rs, src/ui/find_bar.rs</span>`,
      `<span class="c">• </span>Edited <span class="p">src/term/find.rs </span><span class="g">(+38 </span><span class="r">-9</span><span class="g">)</span>`,
      `<span class="c">• </span>Edited <span class="p">src/ui/find_bar.rs </span><span class="g">(+12 </span><span class="r">-2</span><span class="g">)</span>`, '',
    ];
    const ran = `<span class="c">• </span>Ran <span class="p">cargo test --workspace</span>`;
    let lines;
    if (stg === 0) lines = [...head, cxWork(48 + secs(vt))];
    else if (stg === 1) lines = [...head,
      `<span class="y">▌ </span><span class="p">Allow Codex to run this command?</span>`,
      `<span class="y">▌</span>`,
      `<span class="y">▌ </span><span class="d">$ </span>cargo test --workspace`,
      `<span class="y">▌</span>`,
      `<span class="y">▌ </span><span class="b">› 1. Yes, run it</span>`,
      `<span class="y">▌ </span>  2. Yes, and don't ask again for <span class="p">cargo test</span>`,
      `<span class="y">▌ </span>  3. No, and tell Codex what to do differently <span class="d">(esc)</span>`];
    else if (stg === 2) lines = [...head, ran, `<span class="d">  └ Compiling ternarycode v0.2.5</span>`, '', cxWork(secs(ts))];
    else lines = [...head, ran, `<span class="d">  └ </span><span class="g">✓ 214 passed </span><span class="d">(18.4s)</span>`, '',
      `<span class="g">✓ </span>The find bar counts its matches now: “3 of 12”.`, '', `<span class="d">› </span>${CUR}`];
    const st = stOf(stg);
    const note = stg === 1 ? notice('inp', 'Codex wants to run a command', { body: ASK.ternarycode.body, sec: 'Reject', pri: 'Approve', sc: '↵', priAttr: 'data-approve' })
      : stg === 3 ? notice('done', 'Codex finished its turn', { body: '2 files changed · +50 −11', sec: 'Review', pri: 'Commit…' }) : '';
    return {
      bar: { branch: 'main', changed: 2 },
      tabs: [{ a: 'Cx', t: 'Find bar counts its matches', st, on: true }, { t: 'cargo watch', st: 'run' }],
      sessions: [{ a: 'Cx', t: 'Find bar counts its matches', meta: ago(6, vt), st, on: true }, { a: 'SH', t: 'cargo watch', meta: '1h', st: 'run' }],
      lsp: 'rust-analyzer',
      body: hsplit(
        pane({ a: 'Cx', name: 'Find bar counts its matches', st, task: { run: stg === 2 ? 'Running cargo test' : 'Editing src/ui/find_bar.rs', inp: 'cargo test --workspace', done: '2 files · +50 −11' }[st],
          timer: clock(355, vt), profile: 'codex1 · 18%', focus: true, wait: stg === 1, notice: note, lines }),
        pane({ name: 'cargo watch', st: 'run', task: 'Checking ternarycode', lines: [
          `${ps1('ternarycode')}cargo watch -x check`,
          `<span class="y">[Running 'cargo check']</span>`,
          `<span class="g">    Checking </span>ternarycode v0.2.5 (~/code/ternarycode)`,
          `<span class="g">    Finished </span>\`dev\` profile in 3.41s`,
          `<span class="y">[Finished running. Exit status: 0]</span>`] })),
    };
  }

  function apiGateway() {
    const done = S.st['api-gateway'] === 'done';
    const all = [
      `<span class="d">&gt; </span><span class="p">Rate-limit the /v1/search endpoint per API key</span>`, '',
      `<span class="a">⏺ </span><span class="p">Update</span><span class="d">(src/middleware/ratelimit.ts)</span>`,
      `<span class="d">  ⎿  Updated with 41 additions</span>`,
      `<span class="a">⏺ </span><span class="p">Bash</span><span class="d">(npm test)</span>`,
      `<span class="d">  ⎿  </span><span class="g">Tests: 88 passed, 88 total</span>`, '',
      `<span class="a">⏺ </span>Done. /v1/search now allows 60 requests per minute per`,
      `  key, with a Retry-After header on 429.`,
    ];
    const lines = done ? [...all, '', `<span class="d">&gt; </span>${CUR}`] : [...all.slice(0, 5), '', ccSpin('Running the tests', 87 + secs(vt))];
    const st = done ? 'done' : 'run';
    return {
      bar: { branch: 'feat/ratelimit', changed: 2 },
      tabs: [{ a: 'CC', t: 'Rate-limit /v1/search', st, on: true }, { t: 'zsh' }],
      sessions: [{ a: 'CC', t: 'Rate-limit /v1/search', meta: ago(9, vt), st, on: true }, { a: 'SH', t: 'zsh', meta: '' }],
      lsp: 'typescript',
      body: hsplit(
        pane({ a: 'CC', name: 'Rate-limit /v1/search', st, task: done ? '2 files · +41' : 'Bash · npm test', timer: clock(296, done ? S.since['api-gateway'] : vt),
          profile: 'claude2 · 12%', focus: true, notice: done ? notice('done', 'Claude Code finished its turn', { body: '2 files changed · +41', sec: 'Review', pri: 'Commit…' }) : '', lines }),
        pane({ name: 'zsh', place: '~/code/api-gateway', lines: [
          `${ps1('api-gateway', 'feat/ratelimit')}git log --oneline -3`,
          `<span class="y">a41c9e2</span> Add the search endpoint`,
          `<span class="y">7d02b18</span> Cache API keys for 60 s`,
          `<span class="y">3f8e0c1</span> Upgrade to Node 22`,
          `${ps1('api-gateway', 'feat/ratelimit')}${CUR}`] })),
    };
  }

  function mobileApp() {
    return {
      bar: { branch: 'dev', changed: 1 },
      tabs: [{ t: 'flutter test', st: 'fail', on: true }, { t: 'zsh' }],
      sessions: [{ a: 'SH', t: 'flutter test', meta: '4m', st: 'fail', on: true }, { a: 'SH', t: 'zsh', meta: '' }],
      lsp: 'dart',
      body: hsplit(
        pane({ name: 'flutter test', st: 'fail', task: 'exit 1', focus: true, lines: TX.flutterFail,
          notice: notice('fail', 'flutter test exited with code 1', { body: 'after 6s · 1 test failed', sec: 'Ask Claude to fix', pri: 'Run again', dismiss: true }) }),
        pane({ name: 'zsh', place: '~/code/mobile-app', lines: [`${ps1('mobile-app', 'dev')}${CUR}`] })),
    };
  }

  function docsSite() {
    return {
      bar: { branch: 'main', changed: 1, server: 'localhost:4321' },
      tabs: [{ a: 'AG', t: 'Write onboarding copy', on: true }, { t: 'npm run dev', st: 'run' }],
      sessions: [{ a: 'AG', t: 'Write onboarding copy', meta: ago(9, vt), on: true }, { a: 'SH', t: 'npm run dev', meta: ':4321' }],
      lsp: 'astro',
      body: hsplit(
        pane({ a: 'AG', name: 'Write onboarding copy', st: 'idle', stl: 'Active', timer: clock(541, vt), focus: true, lines: [
          `<span class="d">&gt; </span><span class="p">Write the onboarding copy for the docs home page</span>`, '',
          `<span class="c">• </span>Read <span class="p">docs/index.mdx</span>, <span class="p">docs/getting-started.mdx</span>`,
          `<span class="c">• </span>Wrote three hero lines to <span class="p">docs/drafts/hero.md</span>`, '',
          `  1. Every agent, one window.`,
          `  2. Know which agent needs you.`,
          `  3. Your agents, and nothing in the way.`] }),
        pane({ name: 'npm run dev', st: 'run', task: 'localhost:4321', lines: [
          `${ps1('docs-site')}npm run dev`, '',
          `<span class="g"> astro </span> <span class="g">v5.0.3</span> <span class="d">ready in</span> 418 <span class="d">ms</span>`, '',
          ` ┃ Local    <span class="c">http://localhost:4321/</span>`,
          ` ┃ Network  <span class="d">use --host to expose</span>`, '',
          `<span class="d">10:39:52</span> <span class="c">[watch]</span> docs/drafts/hero.md`] })),
    };
  }

  function infra() {
    return {
      bar: { branch: 'main' },
      tabs: [{ t: 'zsh', on: true }],
      sessions: [{ a: 'SH', t: 'zsh', meta: '', on: true }],
      lsp: 'terraform-ls',
      body: hsplit(pane({ name: 'zsh', place: '~/code/infra', focus: true, lines: [
        `${ps1('infra')}terraform plan`,
        `<span class="g">  + </span>aws_s3_bucket.assets`,
        `<span class="y">  ~ </span>aws_cloudfront.cdn`,
        `Plan: 1 to add, 1 to change, 0 to destroy.`,
        `${ps1('infra')}${CUR}`] })),
    };
  }
  const SCENE = { 'storefront': storefront, 'ternarycode': ternarycode, 'api-gateway': apiGateway, 'mobile-app': mobileApp, 'docs-site': docsSite, 'infra': infra };

  /* ------------------------------------------------------------ the window */
  function windowHtml() {
    const q = queue(), sc = SCENE[S.view]();
    const projects = PROJECTS.map(p => ({ ...p, st: S.st[p.n], count: S.st[p.n] === 'inp' ? 1 : 0, on: p.n === S.view,
      sessions: p.n === S.view ? sc.sessions : [], worktrees: p.n === S.view ? sc.worktrees : [] }));
    const waiting = P.filter(p => S.st[p] === 'inp').length;
    const running = 1 + ['storefront', 'ternarycode', 'api-gateway'].filter(p => S.st[p] === 'run').length;
    const proj = PROJECTS.find(p => p.n === S.view);
    return {
      q, html: win({
        sb: sidebar({ projects, inbox: q.length, navAttr: { inbox: 'data-jump' }, usage: { a: 'CC', label: 'Claude · claude1', pct: usagePct(), reset: 'resets 2h 14m' } }),
        bar: projectBar({ p: proj, ...sc.bar }),
        stage: stage(sc.tabs, sc.body),
        status: statusBar({ branch: sc.bar.branch, lsp: sc.lsp, waiting, running, mem: `${308 + Math.floor(vt / 4000) % 6} MB` }),
      }),
    };
  }

  // Spinners keep their phase when the window is drawn again.
  function syncSpin(root) {
    const d = `${-(performance.now() % 1000)}ms`;
    root.querySelectorAll('.k-si.run > i, .coach-spin').forEach(i => { i.style.animationDelay = d; });
  }
  function render(force) {
    const { html } = windowHtml();
    if (!force && html === lastHtml) return;
    lastHtml = html;
    host.innerHTML = html;
    syncSpin(host);
    // a project whose state just changed flashes once
    host.querySelectorAll('.k-prow[data-p]').forEach(el => {
      const p = el.dataset.p;
      if (prevSt[p] !== undefined && prevSt[p] !== S.st[p] && S.st[p] !== 'run') el.classList.add('flash');
    });
    prevSt = { ...S.st };
    renderCoach();
  }

  /* ------------------------------------------------------------ the coach under the window */
  const keys = ks => ks.map(k => `<kbd>${k}</kbd>`).join('');
  const DONE = { 'storefront': 'Claude Code changed 2 files.', 'ternarycode': 'Codex changed 2 files.', 'api-gateway': 'Claude Code changed 2 files.' };
  const SPIN = () => `${I('circle-notch', 'c-run coach-spin')}`;
  function coachState() {
    const cur = S.view, n = nextItem();
    if (S.stage[cur] === 1) return { key: 'ask-' + cur, icon: IF('chat-circle-dots', 'c-wait'),
      text: `<b>${ASK[cur].who}</b> asks to run <code>${ASK[cur].cmd}</code>.`, act: { label: 'Approve', keys: ['↵'], fn: answer } };
    if (n) return { key: 'next-' + n.p + n.st, icon: n.st === 'inp' ? IF('chat-circle-dots', 'c-wait') : IF('check-circle', 'c-done'),
      text: n.st === 'inp' ? `<b>${n.p}</b> needs you. It’s in your Inbox.` : `<b>${n.p}</b> is done. ${DONE[n.p]}`,
      act: { label: n.st === 'inp' ? 'Go there' : 'Take a look', keys: JUMP, fn: jump } };
    const answered = S.stage.storefront === 3 && S.stage.ternarycode === 3 && S.st['api-gateway'] === 'done';
    if (answered) return { key: 'end', icon: IF('check-circle', 'c-done'), end: true,
      text: `That’s the loop. <b>An agent waits, you answer, it gets back to work.</b>`, act: { label: 'Replay', icon: 'arrow-counter-clockwise', fn: replay, ghost: true } };
    if (S.stage.storefront >= 2 || S.stage.ternarycode >= 2) return { key: 'working', icon: SPIN(), text: `Back to work. <b>Nothing else needs you right now.</b>` };
    return { key: 'watch', icon: SPIN(), text: `Five agents at work in six projects. <b>Nothing needs you yet.</b>` };
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
    syncSpin(coachEl);
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
    const wait = coachS.key.startsWith('ask') ? 3800 : 4200;
    if (vt - coachAt > wait && !autoT) {
      const b = coachEl.querySelector('[data-act]');
      if (b) b.classList.add('press');
      autoT = setTimeout(() => { autoT = 0; if (userDrove) return; coachS.act.fn(); render(); }, 260);
    }
  }
  function replay() { clearTimeout(autoT); autoT = 0; EVENTS.forEach(e => { e[1].done = false; }); reset(); render(true); }

  /* ------------------------------------------------------------ input */
  host.addEventListener('click', e => {
    const t = e.target;
    if (t.closest('[data-approve]')) { userDrove = true; answer(); render(); return; }
    if (t.closest('[data-jump]')) { userDrove = true; jump(); render(); return; }
    const el = t.closest('[data-p]');
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

  /* ------------------------------------------------------------ scale, theme & clock */
  const scaler = host.parentElement;
  const fit = () => { const s = scaler.clientWidth / 1440; host.style.transform = `scale(${s})`; scaler.style.height = `${Math.round(900 * s)}px`; };
  new ResizeObserver(fit).observe(scaler);
  fit();
  if (window.matchMedia) {
    const mq = matchMedia('(prefers-color-scheme: light)');
    const redraw = () => render(true);
    mq.addEventListener ? mq.addEventListener('change', redraw) : mq.addListener(redraw);
  }

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
  setInterval(() => { if (!active) return; gi = (gi + 1) % GL.length; host.querySelectorAll('.tspin').forEach(s => { s.textContent = GL[gi]; }); }, 130);

  reset();
  render(true);
  window.tcDemo = { replay };
})();
