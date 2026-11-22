/* Page behaviour: icons, the latest release, the visitor's system, install tabs, copy buttons,
   the shortcut table, frames drawn with the app's design system, and reveal on scroll. */
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const REPO = 'ilhamnadhif/ternarycode';
  const GH = `https://github.com/${REPO}`;
  const PH = window.PH || {};
  const store = {
    get(k) { try { return JSON.parse(sessionStorage.getItem(k)); } catch { return null; } },
    set(k, v) { try { sessionStorage.setItem(k, JSON.stringify(v)); } catch { /* private mode */ } },
  };

  /* ---------------------------------------------------------- icons (Phosphor, by name) */
  window.fillIcons = (root = document) => $$('[data-i]', root).forEach(el => {
    const cp = PH[el.dataset.i];
    if (cp) el.textContent = String.fromCodePoint(cp);
  });
  fillIcons();

  /* ---------------------------------------------------------- nav */
  const nav = $('#nav');
  const onScroll = () => nav.classList.toggle('solid', scrollY > 8);
  addEventListener('scroll', onScroll, { passive: true });
  onScroll();

  /* ---------------------------------------------------------- the visitor's system */
  const ua = navigator.userAgent;
  const plat = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
  const OS = /Win/i.test(plat) || /Windows/.test(ua) ? 'win'
    : /Mac|iPhone|iPad|iPod/i.test(plat) || /Mac OS X|iPhone|iPad/.test(ua) ? 'mac'
    : /Linux|X11|CrOS/i.test(plat) || /Linux|X11/.test(ua) ? (/Android/.test(ua) ? 'mac' : 'linux') : 'mac';
  const NAME = { mac: 'macOS', win: 'Windows', linux: 'Linux' };

  /* ---------------------------------------------------------- release: version, files, sizes */
  const FALLBACK = '0.2.0';
  const FILE = {
    'dmg': v => `TernaryCode-${v}.dmg`,
    'win-setup': v => `TernaryCode-${v}-windows-x64-setup.exe`,
    'win-zip': v => `TernaryCode-${v}-windows-x64.zip`,
    'deb-x64': v => `TernaryCode-${v}-linux-amd64.deb`,
    'deb-arm': v => `TernaryCode-${v}-linux-arm64.deb`,
    'rpm-x64': v => `TernaryCode-${v}-linux-x86_64.rpm`,
    'rpm-arm': v => `TernaryCode-${v}-linux-aarch64.rpm`,
    'appimage-x64': v => `TernaryCode-${v}-linux-x86_64.AppImage`,
    'appimage-arm': v => `TernaryCode-${v}-linux-aarch64.AppImage`,
    'tar-x64': v => `TernaryCode-${v}-linux-x86_64.tar.gz`,
    'tar-arm': v => `TernaryCode-${v}-linux-aarch64.tar.gz`,
  };
  const mb = n => `${Math.max(1, Math.round(n / 1048576))} MB`;
  $$('[data-size]').forEach(el => { el.dataset.base = el.textContent; });
  function applyRelease(v, sizes) {
    $$('[data-version]').forEach(el => { el.textContent = v; });
    $$('[data-release-link]').forEach(a => { a.href = `${GH}/releases/tag/v${v}`; });
    $$('[data-asset]').forEach(a => {
      const name = FILE[a.dataset.asset](v);
      const known = !sizes || name in sizes;
      a.href = known ? `${GH}/releases/download/v${v}/${name}` : `${GH}/releases/latest`;
      const s = $('[data-size]', a);
      if (s && sizes && sizes[name]) s.textContent = `${s.dataset.base} · ${mb(sizes[name])}`;
    });
    winDirect = $('[data-asset="win-setup"]').href;
    setCta();
  }
  let winDirect = '';
  const cached = store.get('tc-release');
  applyRelease(cached ? cached.v : FALLBACK, cached ? cached.sizes : null);
  if (!cached || Date.now() - cached.at > 30 * 60 * 1000) {
    fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: 'application/vnd.github+json' } })
      .then(r => (r.ok ? r.json() : null))
      .then(j => {
        if (!j || !j.tag_name) return;
        const v = String(j.tag_name).replace(/^v/, '');
        const sizes = {};
        (j.assets || []).forEach(a => { sizes[a.name] = a.size; });
        store.set('tc-release', { v, sizes, at: Date.now() });
        applyRelease(v, sizes);
      })
      .catch(() => { /* offline or rate-limited: the links above still work */ });
  }

  /* ---------------------------------------------------------- calls to action */
  function setCta() {
    $$('#cta-main, [data-cta-main]').forEach(a => {
      $('span', a).textContent = `Download for ${NAME[OS]}`;
      a.href = OS === 'win' && winDirect ? winDirect : '#install';
      a.dataset.os = OS;
    });
  }
  const note = $('#cta-note');
  if (note && OS !== 'mac') {
    const others = ['mac', 'win', 'linux'].filter(o => o !== OS);
    note.innerHTML = `Free and open source · also for <a href="#install" data-os-link="${others[0]}">${NAME[others[0]]}</a> and <a href="#install" data-os-link="${others[1]}">${NAME[others[1]]}</a>`;
  }

  /* ---------------------------------------------------------- install tabs */
  const tabs = $$('.tabs-os [role="tab"]');
  function selectTab(os) {
    tabs.forEach(t => {
      const on = t.id === `tab-${os}`;
      t.setAttribute('aria-selected', String(on));
      t.tabIndex = on ? 0 : -1;
      $(`#${t.getAttribute('aria-controls')}`).hidden = !on;
    });
  }
  tabs.forEach((t, i) => {
    t.addEventListener('click', () => selectTab(t.id.slice(4)));
    t.addEventListener('keydown', e => {
      if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
      const n = tabs[(i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length];
      n.focus(); n.click();
    });
  });
  selectTab(OS);
  document.addEventListener('click', e => {
    const a = e.target.closest('[data-os-link], #cta-main, [data-cta-main], [data-dl-nav]');
    if (!a) return;
    const os = a.dataset.osLink || a.dataset.os || OS;
    if (a.getAttribute('href') === '#install') selectTab(os);
  });

  /* ---------------------------------------------------------- copy */
  $$('[data-copy]').forEach(b => b.addEventListener('click', async () => {
    const text = $(`#${b.dataset.copy}`).textContent.trim();
    let ok = false;
    try { await navigator.clipboard.writeText(text); ok = true; } catch {
      const r = document.createRange(); r.selectNodeContents($(`#${b.dataset.copy}`));
      const s = getSelection(); s.removeAllRanges(); s.addRange(r);
      try { ok = document.execCommand('copy'); } catch { ok = false; }
    }
    const label = $('span', b), icon = $('.ic', b);
    label.textContent = ok ? 'Copied' : 'Select it';
    icon.dataset.i = ok ? 'check' : 'copy'; fillIcons(b);
    b.classList.toggle('ok', ok);
    clearTimeout(b._t);
    b._t = setTimeout(() => { label.textContent = 'Copy'; icon.dataset.i = 'copy'; fillIcons(b); b.classList.remove('ok'); }, 1800);
  }));

  /* ---------------------------------------------------------- shortcuts */
  const KEYS = [
    ['jump', '<b>Next waiting agent</b>', ['⌘J'], ['Ctrl+Shift+J']],
    ['back', 'Back to where you were', ['⇧⌘J'], ['Ctrl+Alt+J']],
    ['palette', 'Command palette', ['⌘K'], ['Ctrl+Shift+K']],
    ['project', 'Switch project', ['⌘P'], ['Ctrl+Shift+P']],
    ['run', 'Run an agent', ['⇧⌘A'], ['Ctrl+Shift+A']],
    ['grid', 'Grid', ['⌘0'], ['Ctrl+Shift+0']],
    ['code', 'Code drawer', ['⌘E'], ['Ctrl+Shift+Y']],
    ['review', 'Review changes', ['⇧⌘R'], ['Ctrl+Shift+R']],
    ['commit', 'Commit, push &amp; PR', ['⇧⌘C'], ['Ctrl+Shift+Alt+C']],
    ['browser', 'Browser panel', ['⇧⌘B'], ['Ctrl+Shift+Alt+B']],
    ['split', 'Split right · down', ['⌘D', '⇧⌘D'], ['Ctrl+Shift+D', 'Ctrl+Shift+E']],
    ['find', 'Find · in all panes', ['⌘F', '⇧⌘F'], ['Ctrl+Shift+F', 'Ctrl+Shift+G']],
    ['narrow', 'Narrow sidebar', ['⌥⌘B'], ['Ctrl+Alt+B']],
  ];
  const caps = combo => (combo.includes('+') ? combo.split('+') : [...combo]).map(k => `<kbd>${k}</kbd>`).join('');
  const combos = list => list.map(caps).join('<span style="color:var(--fg-4);margin:0 4px">·</span>');
  let keysOs = OS === 'mac' ? 'mac' : 'pc';
  function drawKeys() {
    const col = keysOs === 'mac' ? 2 : 3;
    $('#keygrid').innerHTML = KEYS.map(k => `<div class="krow"><span>${k[1]}</span><span class="kk">${combos(k[col])}</span></div>`).join('');
    $('#keys-note').innerHTML = keysOs === 'mac'
      ? 'The command palette, <kbd>⌘</kbd><kbd>K</kbd>, lists all of them.'
      : 'On Windows and Linux, <kbd>⌘</kbd> is <kbd>Ctrl</kbd><kbd>Shift</kbd>, so <kbd>Ctrl</kbd><kbd>C</kbd> still reaches the terminal. <kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>K</kbd> lists them all.';
    $$('.seg-os button').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.keys === keysOs)));
    // the shortcuts quoted in the text follow the same choice
    $$('[data-k]').forEach(el => { const k = KEYS.find(r => r[0] === el.dataset.k); if (k) el.innerHTML = combos(k[col]); });
  }
  $$('.seg-os button').forEach(b => b.addEventListener('click', () => { keysOs = b.dataset.keys; drawKeys(); }));
  drawKeys();

  /* ---------------------------------------------------------- frames drawn with the app's kit */
  function frame(el, w, h, html) {
    const sc = $('.scaled', el);
    sc.innerHTML = html;
    const fit = () => { const s = el.clientWidth / w; sc.style.transform = `scale(${s})`; el.style.height = `${Math.round(h * s)}px`; };
    new ResizeObserver(fit).observe(el);
    fit();
  }
  function reviewWindow() {
    const groups = [
      { name: 'Active', n: 6, sum: SUM(), rows: [
        { n: 'docs-site', st: 'run', a: 'SH', stl: 'npm run dev', dots: ['r'] },
        { n: 'ml-pipeline', st: 'run', a: 'Cx', stl: 'Codex · 62%', dots: ['r'] },
        { n: 'mobile-app', st: 'wait', badge: ['wait', 1], dots: ['w', ''] },
        { n: 'storefront', st: 'run', a: 'CC', stl: 'Claude Code', dots: ['r', 'web'] },
        { n: 'storefront', child: true, label: 'cart-badge', br: 'tc/cart-badge', chg: 5, st: 'done', sel: true },
        { n: 'ternary-engine', st: 'wait', badge: ['wait', 1], dots: ['w'] },
      ] },
      { name: 'Idle', n: 3, open: false, rows: [] },
    ];
    const top = topbar({
      left: `${ib('arrow-left', { sm: true })}<span class="ttl"><span class="nm">Review</span></span>
        <span class="row g6" style="font-size:12.5px">${pt('storefront', 's18')}<span class="t2">storefront</span>${I('caret-right', 't4')}${I('git-fork', 't3')}<span class="medium">cart-badge</span></span>
        ${vsep()}<span class="chip line">${I('git-branch')}<span>Since branched from <span class="mono" style="font-size:11px">main</span></span>${I('caret-down', 'ext')}</span>`,
      right: `<span class="mono t3" style="font-size:11px">5 files · <span class="c-add">+112</span> <span class="c-del">−1</span></span>
        ${seg(['Unified', 'Split'], 1)}${vsep()}<span class="btn ghost">Close <span class="kb">esc</span></span>`,
    });
    const commits = [
      { h: '', t: 'Not committed', d: '1 file · header.css', wip: true },
      { h: 'a41c9e2', t: 'Add CartBadge to the header', d: 'Claude Code · 6 min' },
      { h: '7d02b18', t: 'Add useCart hook', d: 'Claude Code · 18 min' },
    ];
    const left = `<div class="cd-col" style="width:300px">
      <div class="cd-sh">Commits <span class="n">2 + not committed</span><span class="sp"></span><span class="t3" style="font-size:11.5px">All</span></div>
      <div class="cd-commits">${commits.map(c => `<div class="cd-cm ${c.wip ? 'wip' : ''}"><span class="cd-dot"></span><div style="min-width:0;flex:1"><div class="ell">${c.t}</div><div class="t3" style="font-size:11px;margin-top:1px">${c.h ? `<span class="mono">${c.h}</span> · ` : ''}${c.d}</div></div></div>`).join('')}
        <div class="cd-cm base"><span class="cd-dot"></span><div class="t3" style="font-size:11.5px">branched from <span class="mono">main</span> at <span class="mono">3f8e0c1</span> · 41 min</div></div></div>
      <div class="cd-sh" style="margin-top:6px">Files <span class="n">5</span></div>
      <div class="cd-flist">${cdFiles([
        { m: 'a', n: 'CartBadge.tsx', d: 'src/components', a: 31 },
        { m: 'm', n: 'Header.tsx', d: 'src/components', a: 7, x: 1, on: true },
        { m: 'a', n: 'useCart.ts', d: 'src/cart', a: 22 },
        { m: 'a', n: 'cart.test.tsx', d: 'src/cart', a: 40 },
        { m: 'm', n: 'header.css', d: 'src/styles', a: 12 },
      ])}</div>
    </div>`;
    const body = sheet(`
      <div class="row" style="flex:1;min-height:0;align-items:stretch">
        ${left}
        <div class="cd-main">
          <div class="cd-dfh">${cdMark('m')}<span class="medium">src/components/Header.tsx</span><span class="mono c-add" style="font-size:11px">+7</span><span class="mono c-del" style="font-size:11px">−1</span>${bars(7, 1)}<span class="sp"></span><span class="mono t3" style="font-size:10.5px">in a41c9e2</span>${btn('Open in Editor', { sm: true, i: 'arrow-square-out', kb: '↵' })}</div>
          <div class="cd-sphd"><span class="mono">main · 3f8e0c1</span><span class="mono">tc/cart-badge</span></div>
          ${cdSplit(HEADER_SPLIT, 'ts')}
          <div class="cd-next">${cdMark('m')}<span class="medium">src/styles/header.css</span><span class="mono c-add" style="font-size:11px">+12</span>${bars(12, 0)}<span class="sp"></span><span class="t3" style="font-size:11.5px">${kbd('j')} next file</span></div>
        </div>
      </div>
      <div class="cd-wtf">
        ${I('git-fork', 't3')}<span class="mono" style="font-size:11px">tc/cart-badge</span><span class="t3">2 commits ahead of <span class="mono">main</span>, nothing new on <span class="mono">main</span>, 1 file not committed</span>
        <span class="sp"></span>${btn('Remove Worktree…', { k: 'ghost', i: 'trash', cls: 'dangerq' })}${btn('Merge into main…', { k: 'primary', i: 'git-merge' })}
      </div>`, { cls: 'focus', style: 'flex:1' });
    return win({ theme: 'dark', sb: sidebar({ sel: null, groups }), top, main: body,
      status: statusbar({ lead: `<span class="lead">${IF('bell-ringing', 'c-wait')}<b>mobile-app</b><span class="t3">·</span>Claude Code wants to run <span class="mono" style="font-size:10.5px;color:var(--text)">flutter test</span><span class="t3">+1 more</span>${kbd('⌘J')}</span>` }) });
  }
  // The commit box under Changes: the message from the agent's task and last words, one button.
  function shipWindow() {
    const shFile = r => `<div class="sh-fr ${r.on ? 'on' : ''}">${chk(true)}${cdMark(r.m)}
      <div class="sh-nm"><div class="ell t1">${r.n}</div><div class="mono t3 ell" style="font-size:10.5px;margin-top:1px">${r.d}</div></div>
      ${al(r.ag, 11)}<span class="cd-pm"><span class="c-add">+${r.a}</span>${r.x ? `<span class="c-del">−${r.x}</span>` : ''}</span></div>`;
    const files = [
      { m: 'a', n: 'CartBadge.tsx', d: 'src/components', a: 31, ag: 'CC' },
      { m: 'a', n: 'useCart.ts', d: 'src/cart', a: 22, ag: 'CC' },
      { m: 'm', n: 'Header.tsx', d: 'src/components', a: 7, x: 1, ag: 'CC', on: true },
    ];
    const unified = [`<div class="hunk"><span>@@ −1,14 +1,19 @@</span><span class="ctx">export function Header()</span></div>`];
    for (const [t, a, l, b, r] of HEADER_SPLIT) {
      if (t === 'add') unified.push(cdDl('add', '', b, r, 'ts'));
      else if (t === 'mod') { unified.push(cdDl('del', a, '', l, 'ts')); unified.push(cdDl('add', '', b, r.replace('<CartButton>', '<CartButton⟦⟧>'), 'ts')); }
      else unified.push(cdDl('', a, b, l, 'ts'));
    }
    const agent = [
      ...T.ccHead('opus 4.6', '~/code/storefront'),
      `<span class="d">&gt;</span> Add a cart badge to the header, then`,
      `  check it in the browser`,
      '',
      `<span class="g">⏺</span> Write <span class="b">src/components/CartBadge.tsx</span> <span class="g">+31</span>`,
      `<span class="g">⏺</span> Write <span class="b">src/cart/useCart.ts</span> <span class="g">+22</span>`,
      `<span class="g">⏺</span> Update <span class="b">src/components/Header.tsx</span> <span class="g">+7</span> <span class="r">−1</span>`,
      `<span class="g">⏺</span> <span class="b">ternarycode</span> <span class="d">·</span> browser_click <span class="d">× 2</span>`,
      `  <span class="f">⎿</span>  <span class="d">The badge shows 2.</span>`,
      '',
      `<span class="g">⏺</span> The header shows the item count from`,
      `  useCart. Checked in the browser: 0 → 1 → 2.`,
      '',
      ...T.prompt(48),
    ];
    const box = `<div class="sh-box">
      <div class="sh-msg"><div class="t">Add a cart badge to the header</div><div class="b">The header shows the item count from useCart. Checked in the browser: 0 → 1 → 2.</div>${ib('sparkle', { xs: true, cls: 'ai' })}</div>
      <div class="sh-src">${al('CC', 10)}<span>From Claude Code’s task and its last words</span></div>
      <div class="sh-go"><span class="btn primary m">${I('git-pull-request')}Commit, Push &amp; Create PR<span class="kb">⌘↵</span></span><span class="btn primary x">${I('caret-down')}</span></div>
      <div class="sh-to">${I('git-branch')}<span>feature/cart-badge</span><span class="ar">→</span><span class="ell">origin · PR into main</span></div>
    </div>`;
    const scope = `<div class="cd-scope">
      <span class="cd-sc on"><span class="cd-sci">${al('CC', 12)}</span><span class="cd-scb"><span>Since Claude Code started</span><span class="cd-cnt">6 min ago · 3 files</span></span></span>
      <span class="cd-sc"><span class="cd-sci">${I('git-commit')}</span><span class="cd-scb"><span>Since last commit</span><span class="cd-cnt">4 files</span></span></span>
    </div>`;
    const drawer = sheet(`
      ${cdHead(1, { n: 4 })}
      <div class="cd-body">
        <div class="cd-col sh-col">${scope}<div class="cd-flist">${files.map(shFile).join('')}</div>${box}</div>
        <div class="cd-main">
          <div class="cd-dfh">${cdMark('m')}<span class="medium">src/components/Header.tsx</span><span class="mono c-add" style="font-size:11px">+7</span><span class="mono c-del" style="font-size:11px">−1</span>${bars(7, 1)}<span class="sp"></span>${seg(['Unified', 'Split'], 0, 'sm')}${ib('arrow-square-out', { sm: true })}</div>
          <div class="cd-diff code">${unified.join('')}</div>
          <div class="cd-sl"><span class="row g4">${kbd('j')}${kbd('k')}<span>file</span></span><span class="row g4">${kbd('space')}<span>in or out of the commit</span></span><span class="sp"></span><span class="row g4">${kbd('⇧⌘C')}<span>commit box</span></span></div>
        </div>
      </div>`, { cls: 'focus cd-drawer', style: 'flex:1' });
    const groups = GROUPS('storefront').map(g => ({ ...g, rows: g.rows.map(r => r.n === 'storefront' ? { ...r, st: 'done', a: null, stl: 'Done', br: 'feature/cart-badge', chg: 4, dots: ['d', 'r'] } : r) }));
    return win({
      theme: 'dark',
      sb: sidebar({ sel: 'storefront', queue: QUEUE.slice(0, 2), groups }),
      top: topbar({ p: 'storefront', br: 'feature/cart-badge', chg: 4, code: true, tabs: [{ al: 'CC', l: 'Claude Code', on: true }, { l: 'zsh', i: 'terminal-window' }] }),
      main: `${pane({ a: 'CC', st: 'done', stl: 'Done', meta: 'opus 4.6', hooks: true, time: '6:12', style: 'width:440px;flex:none' }, agent)}${drawer}`,
    });
  }
  if (typeof LOGO === 'object') $$('[data-logo]').forEach(el => { el.innerHTML = LOGO[el.dataset.logo](16); });
  if (typeof win === 'function') {
    const sb = $('[data-frame="sidebar"]');
    if (sb) frame(sb, 248, 520, `<div class="app dark"><div class="sbx" style="width:248px;height:520px">${sidebar({})}</div></div>`);
    const rv = $('[data-frame="review"]');
    if (rv) frame(rv, 1440, 900, reviewWindow());
    const sh = $('[data-frame="ship"]');
    if (sh) frame(sh, 1440, 900, shipWindow());
    // how each state looks where you meet it: a Grid tile's footer, a sidebar row
    const SPEC = {
      wait: () => `<div class="tile-f wait">${IF('bell-ringing', 'c-wait')}<span class="ell">Claude Code wants to run <code>flutter test</code></span><span class="sp"></span>${btn('Allow', { sm: true, k: 'primary' })}</div>`,
      run: () => prow({ n: 'storefront', st: 'run', a: 'CC', stl: 'Claude Code', dots: ['r', 'r', 'web'] }),
      done: () => `<div class="tile-f done">${IF('check-circle', 'c-done')}<span class="ell">Done · <span class="mono">2 files changed</span></span><span class="sp"></span>${btn('Review', { sm: true, i: 'git-diff' })}</div>`,
      fail: () => `<div class="tile-f fail">${IF('x-circle', 'c-fail')}<span class="ell"><code>npm test</code> failed · exit 1</span><span class="sp"></span>${btn('Show', { sm: true })}</div>`,
    };
    $$('[data-spec]').forEach(el => { el.innerHTML = `<div class="app dark">${SPEC[el.dataset.spec]()}</div>`; });
  }

  /* ---------------------------------------------------------- the lattice behind the hero */
  // Lights on the dot field run along the "</" of the icon, like the app's Running glyph.
  const GLYPH_RUN = { 2: 0, 4: .3, 8: .6, 3: 1.2, 5: 1.5 };
  function lattice(field, n, avoid) {
    if (!field) return;
    const W = field.clientWidth, H = field.clientHeight, step = 24;
    const cols = Math.floor(W / 2 / step);
    const used = [];
    const kinds = ['run', 'run', 'wait', 'run', 'done', 'run', 'run', 'wait', 'run', 'run'];
    let tries = 0;
    field.querySelectorAll('.glyph').forEach(g => g.remove());
    for (let i = 0; i < n && tries < 400; tries++) {
      const cx = (Math.floor(Math.random() * (cols * 2 - 2)) - cols + 1);
      const cy = 1 + Math.floor(Math.random() * Math.max(1, Math.floor(H / step) - 4));
      const x = W / 2 + cx * step, y = 12 + cy * step;
      if (x < 16 || x + 64 > W || y + 64 > H) continue;
      if (avoid && avoid(x, y, W, H)) continue;
      if (used.some(([ux, uy]) => Math.abs(ux - x) < 150 && Math.abs(uy - y) < 110)) continue;
      used.push([x, y]);
      const g = document.createElement('span');
      const k = kinds[i % kinds.length];
      g.className = `glyph ${k}`;
      g.style.cssText = `left:${x - 1.5}px;top:${y - 1.5}px;--c:var(--${k})`;
      g.innerHTML = '<b></b>'.repeat(9);
      const off = Math.random() * 2.4, base = k === 'run' ? GLYPH_RUN : {};
      [...g.children].forEach((d, j) => { if (base[j + 1] !== undefined) d.style.animationDelay = `${base[j + 1] - off}s`; });
      field.appendChild(g);
      i++;
    }
  }
  const heroAvoid = (x, y, W) => Math.abs(x + 25 - W / 2) < Math.min(470, W * .36) || y > 600;
  const drawFields = () => {
    const small = innerWidth < 760;
    // on a phone the text spans the width: lights only in the band above the headline
    lattice($('#field-hero'), small ? 3 : 9, small ? (x, y) => y > 150 : heroAvoid);
    lattice($('#field-final'), small ? 0 : 7, (x, y, W, H) => Math.abs(x + 25 - W / 2) < Math.min(380, W * .3) && Math.abs(y - H / 2) < 260);
  };
  drawFields();
  let lastW = innerWidth;
  addEventListener('resize', () => { if (Math.abs(innerWidth - lastW) > 80) { lastW = innerWidth; drawFields(); } });

  /* ---------------------------------------------------------- reveal on scroll */
  if ('IntersectionObserver' in window) {
    const io = new IntersectionObserver(es => es.forEach(e => { if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); } }), { rootMargin: '0px 0px -8% 0px' });
    $$('.rv').forEach(el => io.observe(el));
  } else $$('.rv').forEach(el => el.classList.add('in'));

  /* ---------------------------------------------------------- demo replay */
  const rp = $('#demo-replay');
  if (rp) rp.addEventListener('click', () => window.tcDemo && window.tcDemo.replay());
})();
