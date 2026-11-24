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
  const FALLBACK = '0.3.12';
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
    $$('[data-release-link]').forEach(a => { a.href = sizes ? `${GH}/releases/tag/v${v}` : `${GH}/releases/latest`; });
    $$('[data-asset]').forEach(a => {
      const name = FILE[a.dataset.asset](v);
      const known = Boolean(sizes && name in sizes);
      a.href = known ? `${GH}/releases/download/v${v}/${name}` : `${GH}/releases/latest`;
      const s = $('[data-size]', a);
      if (s && known && sizes[name]) s.textContent = `${s.dataset.base} · ${mb(sizes[name])}`;
    });
    const winHref = $('[data-asset="win-setup"]').href;
    winDirect = sizes && winHref.includes('/releases/download/') ? winHref : '';
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

  /* ---------------------------------------------------------- shortcuts (the app's own table: src/ui/kit.rs key_labels) */
  const KEYS = [
    ['<b>Command palette</b> · switch project', ['⌘K', '⌘P'], ['Ctrl+Shift+K', 'Ctrl+Shift+P']],
    ['<b>Next waiting agent</b> · back', ['⌘J', '⇧⌘J'], ['Ctrl+Shift+J', 'Ctrl+Alt+J']],
    ['Run an agent', ['⇧⌘A'], ['Ctrl+Shift+A']],
    ['Overview', ['⌘0'], ['Ctrl+Shift+0']],
    ['Changes · Files · Browser', ['⇧⌘R', '⌘E', '⇧⌘B'], ['Ctrl+Shift+R', 'Ctrl+Shift+Y', 'Ctrl+Shift+Alt+B']],
    ['Show or hide the panel · fill the window', ['⇧⌘L', '⇧⌘E'], ['Ctrl+Shift+L', 'Ctrl+Alt+E']],
    ['Commit, push &amp; pull request', ['⇧⌘C'], ['Ctrl+Shift+Alt+C']],
    ['Open a file', ['⌘O'], ['Ctrl+Shift+O']],
    ['New tab · close pane', ['⌘T', '⌘W'], ['Ctrl+Shift+T', 'Ctrl+Shift+W']],
    ['Split right · down', ['⌘D', '⇧⌘D'], ['Ctrl+Shift+D', 'Ctrl+Shift+E']],
    ['Find · in every pane', ['⌘F', '⇧⌘F'], ['Ctrl+Shift+F', 'Ctrl+Shift+G']],
    ['Sidebar · narrow sidebar', ['⌘B', '⌥⌘B'], ['Ctrl+Shift+B', 'Ctrl+Alt+B']],
    ['Zen: only the terminals', ['⌥⌘Z'], ['Ctrl+Alt+Z']],
    ['Project 1–9 · move project', ['⌘1–9', '⌥⌘↑', '⌥⌘↓'], ['Ctrl+Shift+1–9', 'Ctrl+Alt+↑', 'Ctrl+Alt+↓']],
  ];
  // The shortcuts quoted in the text: <span data-k="jump">.
  const INLINE = {
    jump: ['⌘J', 'Ctrl+Shift+J'], back: ['⇧⌘J', 'Ctrl+Alt+J'], palette: ['⌘K', 'Ctrl+Shift+K'], run: ['⇧⌘A', 'Ctrl+Shift+A'],
    overview: ['⌘0', 'Ctrl+Shift+0'], changes: ['⇧⌘R', 'Ctrl+Shift+R'], panel: ['⇧⌘L', 'Ctrl+Shift+L'], expand: ['⇧⌘E', 'Ctrl+Alt+E'],
    browser: ['⇧⌘B', 'Ctrl+Shift+Alt+B'], commit: ['⇧⌘C', 'Ctrl+Shift+Alt+C'], zen: ['⌥⌘Z', 'Ctrl+Alt+Z'], open: ['⌘O', 'Ctrl+Shift+O'],
    move: ['⌥⌘↑', 'Ctrl+Alt+↑'], project: ['⌘1–9', 'Ctrl+Shift+1–9'], settings: ['⌘,', 'Ctrl+Shift+,'],
  };
  // ⌥⌘↑ → ⌥ ⌘ ↑ ; Ctrl+Alt+↑ → Ctrl Alt ↑
  const split = c => (c.includes('+') ? c.split('+') : (c.match(/^[⌘⇧⌥⌃]+/) || [''])[0].split('').concat(c.replace(/^[⌘⇧⌥⌃]+/, '')).filter(Boolean));
  const caps = combo => `<span class="kc">${split(combo).map(k => `<kbd>${k}</kbd>`).join('')}</span>`;
  const combos = list => list.map(caps).join('<span class="kdot">·</span>');
  let keysOs = OS === 'mac' ? 'mac' : 'pc';
  function drawKeys() {
    const col = keysOs === 'mac' ? 1 : 2;
    $('#keygrid').innerHTML = KEYS.map(k => `<div class="krow"><span>${k[0]}</span><span class="kk">${combos(k[col])}</span></div>`).join('');
    $('#keys-note').innerHTML = keysOs === 'mac'
      ? 'The command palette, <kbd>⌘</kbd><kbd>K</kbd>, lists all of them.'
      : 'On Windows and Linux, <kbd>⌘</kbd> is <kbd>Ctrl</kbd><kbd>Shift</kbd> and <kbd>⌥</kbd> stays <kbd>Alt</kbd>, so <kbd>Ctrl</kbd><kbd>C</kbd> still reaches the terminal. <kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>K</kbd> lists them all.';
    $$('.seg-os button').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.keys === keysOs)));
    // the shortcuts quoted in the text follow the same choice
    $$('[data-k]').forEach(el => { const k = INLINE[el.dataset.k]; if (k) el.innerHTML = caps(k[col - 1]); });
  }
  $$('.seg-os button').forEach(b => b.addEventListener('click', () => { keysOs = b.dataset.keys; drawKeys(); }));
  drawKeys();

  /* ---------------------------------------------------------- frames drawn with the app's kit */
  function frame(el, w, h, html) {
    const sc = $('.scaled', el);
    sc.innerHTML = html;
    const fit = () => { const s = el.clientWidth / w; sc.style.transform = `scale(${s})`; el.style.height = `${Math.round(h * s)}px`; };
    if (!el._fit) { el._fit = fit; new ResizeObserver(() => el._fit()).observe(el); }
    fit();
  }
  if (typeof LOGO === 'object') $$('[data-logo]').forEach(el => { el.innerHTML = LOGO[el.dataset.logo](16); });
  function drawKit() {
    if (typeof sidebar !== 'function') return;
    const th = THEME();
    const sb = $('[data-frame="sidebar"]');
    if (sb) {
      const projects = PROJECTS.map(p => ({ ...p, on: p.n === 'storefront' }));
      Object.assign(projects[0], { st: 'inp', count: 1,
        sessions: [{ a: 'CC', t: 'Fix OAuth redirect loop', meta: '2m', st: 'inp', on: true }, { a: 'Cx', t: 'Add order export to CSV', meta: '14m', st: 'run' }, { a: 'SH', t: 'npm run dev', meta: ':5173' }],
        worktrees: [{ n: 'codex-export', br: 'tc/export', st: 'run' }] });
      projects[1].st = 'run'; projects[2].st = 'done'; projects[3].st = 'fail';
      frame(sb, 248, 660, `<div class="app ${th}"><div class="k-win" style="width:248px;height:660px">${sidebar({ projects, inbox: 3,
        usage: { a: 'CC', label: 'Claude · claude1', pct: 42, reset: 'resets 2h 14m' } })}</div></div>`);
    }
    // how each state looks where you meet it: a Notice at the foot of the pane, a pane header
    const SPEC = {
      wait: () => notice('inp', 'Claude Code wants to run a command', { body: 'Bash · npm test -- auth', cls: 'solo' }),
      run: () => `<div class="k-sheet" style="border-radius:9px">${paneHead({ a: 'Cx', name: 'Add order export to CSV', st: 'run', task: 'Editing api/orders.ts', timer: '2:14' })}</div>`,
      done: () => notice('done', 'Codex finished its turn', { body: '3 files changed · +102 −2', sec: 'Review', pri: 'Commit…', cls: 'solo' }),
      fail: () => notice('fail', 'Exited with code 1', { body: 'pnpm build · after 42s', pri: 'Run again', dismiss: true, cls: 'solo' }),
      limit: () => notice('lim', 'claude1 hit its 5-hour limit', { body: 'Resets in 14m · claude2 is ready', pri: 'Resume on claude2', cls: 'solo' }),
      resume: () => notice('info', 'Claude Code ran here before the restart', { body: '“Review pass” · 41 min ago', pri: 'Resume', cls: 'solo' }),
      server: () => `<span class="k-chip"><span class="k-live"></span>localhost:5173${I('arrow-up-right', 'k-ext')}</span><span class="k-chip">${I('git-pull-request', 'k-acc-i')}<span class="k-t1">#128</span>${si('done')}<span class="k-sans">checks passed</span></span>`,
      usage: () => usagePop([
        { a: 'CC', id: 'claude1', plan: 'MAX', st: 'lim', stl: 'Limited', p5: 100, pw: 61, reset: '↺ 14m' },
        { a: 'CC', id: 'claude2', plan: 'PRO', st: 'done', stl: 'Ready', p5: 12, pw: 30, reset: '↺ 4h 2m' },
        { a: 'Cx', id: 'codex1', plan: 'PLUS', st: 'run', stl: 'In use', p5: 18, pw: 22, reset: '↺ 2h 40m' }]),
    };
    $$('[data-spec]').forEach(el => { el.innerHTML = `<div class="app ${th}">${SPEC[el.dataset.spec]()}</div>`; });
  }
  drawKit();
  if (window.matchMedia) {
    const mq = matchMedia('(prefers-color-scheme: light)');
    mq.addEventListener ? mq.addEventListener('change', drawKit) : mq.addListener(drawKit);
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
