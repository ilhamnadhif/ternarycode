/* Slate components: plain functions that return HTML, composed the way the app composes its window
   (spec: design/figcraft kit.py and components.py). app.css holds the tokens and the k- classes. */

const I = (n, cls = '') => { const cp = window.PH[n]; if (!cp) { console.warn('icon', n); return ''; } return `<i class="ph ${cls}">&#x${cp.toString(16)};</i>`; };
const IF = (n, cls = '') => { const cp = window.PH[n]; if (!cp) { console.warn('icon', n); return ''; } return `<i class="phf ${cls}">&#x${cp.toString(16)};</i>`; };
const esc = s => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
// The page follows the system's appearance, and so does every window drawn on it.
const THEME = () => (window.matchMedia && matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark');

/* ------------------------------------------------------------------ agent marks */
const LOGO = {
  CC: c => `<svg viewBox="0 0 24 24" width="${c}" height="${c}"><path fill="#D97757" d="m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z"/></svg>`,
  Cx: c => `<svg viewBox="0 0 24 24" width="${c}" height="${c}"><path fill="currentColor" d="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z"/></svg>`,
  OC: c => `<svg viewBox="0 0 240 300" width="${Math.round(c * 0.8)}" height="${c}"><path fill="currentColor" d="M180 60H60V240H180V60ZM240 300H0V0H240V300Z"/><path fill="currentColor" fill-opacity=".28" d="M180 240H60V120H180V240Z"/></svg>`,
  AG: c => `<img src="assets/img/agents/antigravity.png" width="${c}" height="${c}" alt="">`,
};
const AGN = { CC: 'Claude Code', Cx: 'Codex', OC: 'OpenCode', AG: 'Antigravity CLI', SH: 'zsh' };
// An agent's mark on its small tile; a shell shows >_.
const av = (a, size = '') => `<span class="k-av ${size}">${a === 'SH' || !LOGO[a] ? I('terminal') : LOGO[a](24)}</span>`;
// A project's letter on its hue.
const pa = (p, size = '') => `<span class="k-pa ${p.hue} ${size}">${p.letter}</span>`;

/* ------------------------------------------------------------------ status language */
// One state, one glyph, one colour, one word, in every place it shows.
const ST = {
  run:   { l: 'Running', g: 'circle-notch' },
  inp:   { l: 'Needs input', g: 'chat-circle-dots', f: 1 },
  done:  { l: 'Done', g: 'check-circle', f: 1 },
  fail:  { l: 'Failed', g: 'x-circle', f: 1 },
  idle:  { l: 'Idle', g: 'circle' },
  lim:   { l: 'Rate limited', g: 'hourglass-medium', f: 1 },
  queue: { l: 'Queued', g: 'clock' },
};
const si = (st, size = '') => { const s = ST[st]; return `<span class="k-si ${st} ${size}">${s.f ? IF(s.g) : I(s.g)}</span>`; };
const pill = (st, label) => `<span class="k-pill ${st}">${si(st)}${label ?? ST[st].l}</span>`;

/* ------------------------------------------------------------------ primitives */
const kbd = k => `<span class="k-kbd">${k}</span>`;
// o: {k: 'pri'|'sec'|'ghost', s: small, i: icon, sc: shortcut, attr}
const btn = (label, o = {}) => `<span class="k-btn b-${o.k || 'sec'} ${o.s ? 's' : ''}" ${o.attr || ''}>${o.i ? I(o.i) : ''}${label}${o.sc ? `<span class="k-sc">${o.sc}</span>` : ''}</span>`;
const ib = (n, o = {}) => `<span class="k-ibtn ${o.s ? 's' : ''} ${o.on ? 'on' : ''}">${I(n)}</span>`;
const track = (pct, cls = '') => `<span class="k-track"><i class="${cls}" style="width:${Math.max(0, Math.min(100, pct))}%"></i></span>`;

/* ------------------------------------------------------------------ notice */
const NOTICE = { inp: ['chat-circle-dots', 1], fail: ['x-circle', 1], done: ['check-circle', 1], info: ['clock-counter-clockwise', 0], lim: ['hourglass-medium', 1], neutral: ['power', 0] };
// o: {body, pri, sec, dismiss, glyph, cls, priAttr}
function notice(tone, title, o = {}) {
  const [g, f] = NOTICE[tone], gl = o.glyph || g;
  return `<div class="k-notice ${tone} ${o.cls || ''}">${f && !o.glyph ? IF(gl) : I(gl)}
    <span class="k-nt"><b>${title}</b>${o.body ? `<span>${o.body}</span>` : ''}</span>
    <span class="k-acts">${o.sec ? btn(o.sec, { s: true, attr: o.secAttr }) : ''}${o.pri ? btn(o.pri, { k: 'pri', s: true, sc: o.sc, attr: o.priAttr }) : ''}${o.dismiss ? `<span class="k-dis">${I('x')}</span>` : ''}</span></div>`;
}

/* ------------------------------------------------------------------ sidebar */
// projects: [{n, letter, hue, st, count, on, sessions: [{a, t, meta, st, on}], worktrees: [{n, br, st}]}]
function sidebar(o = {}) {
  const rows = (o.projects || []).map(p => {
    const row = `<div class="k-prow ${p.on ? 'on' : ''}" data-p="${p.n}">${pa(p)}<span class="k-nm">${p.n}</span>${p.count ? `<span class="k-count">${p.count}</span>` : ''}${p.st ? si(p.st) : ''}</div>`;
    if (!p.on) return row;
    const ss = (p.sessions || []).map(s => `<div class="k-srow ${s.on ? 'on' : ''}"><span class="k-rail"></span>${av(s.a)}<span class="k-nm">${s.t}</span><span class="k-meta">${s.meta || ''}</span>${s.st ? si(s.st) : ''}</div>`);
    const wt = (p.worktrees || []).map(w => `<div class="k-srow"><span class="k-rail"></span>${I('git-fork')}<span class="k-nm">${w.n}</span><span class="k-meta">${w.br}</span>${w.st ? si(w.st) : ''}</div>`);
    return row + ss.join('') + wt.join('');
  }).join('');
  const nav = (key, icon, label, right) => `<div class="k-navi ${o.nav === key ? 'on' : ''}" ${o.navAttr && o.navAttr[key] ? o.navAttr[key] : ''}>${I(icon)}<span class="k-sp">${label}</span>${right}</div>`;
  const u = o.usage;
  return `<aside class="k-sb">
    <div class="k-sb-title"><span class="k-lights"><b></b><b></b><b></b></span><span class="k-sp"></span><span class="k-ib">${I('sidebar-simple')}</span></div>
    <div class="k-search">${I('magnifying-glass')}<span class="k-sp">Search…</span>${kbd('⌘K')}</div>
    <div class="k-nav">
      ${nav('inbox', 'tray', 'Inbox', o.inbox ? `<span class="k-count">${o.inbox}</span>` : '')}
      ${nav('overview', 'squares-four', 'Overview', '<span class="k-sc">⌘0</span>')}
    </div>
    <div class="k-projects">
      <div class="k-sec"><span class="k-lb">Projects</span><span class="k-n">${(o.projects || []).length}</span><span class="k-sp"></span><span class="k-act">${I('magnifying-glass')}</span><span class="k-act">${I('plus')}</span></div>
      ${rows}
    </div>
    <span class="k-sp"></span>
    ${u ? `<div class="k-usage"><div class="k-u1">${av(u.a || 'CC')}<b>${u.label}</b><span class="k-mono k-t2" style="font-size:11px">${u.pct}%</span></div>${track(u.pct, u.pct >= 90 ? 'lim' : '')}<div class="k-u3"><span>${u.window || '5h'} window</span><span>${u.reset}</span></div></div>` : ''}
    <div class="k-sb-foot"><span class="k-set">${I('gear-six')}Settings</span><span class="k-sp"></span><span class="k-sc">⌘,</span></div>
  </aside>`;
}

/* ------------------------------------------------------------------ project bar */
// o: {p, branch, changed, pr: [num, 'running'|'passed'|'failed'], server, panel}
function projectBar(o) {
  const PR = { running: [si('run'), 'checks running'], passed: [si('done'), 'checks passed'], failed: [si('fail'), 'check failed'] };
  const pr = o.pr ? `<span class="k-chip">${I('git-pull-request', 'k-acc-i')}<span class="k-t1">#${o.pr[0]}</span>${PR[o.pr[1]][0]}<span class="k-sans">${PR[o.pr[1]][1]}</span></span>` : '';
  const tog = [['changes', 'git-diff'], ['files', 'folder-simple'], ['history', 'clock-counter-clockwise'], ['browser', 'globe-simple']]
    .map(([k, i]) => ib(i, { on: o.panel === k })).join('');
  return `<header class="k-pbar">
    <span class="k-switch">${pa(o.p)}${o.p.n}${I('caret-down', 'k-car')}</span>
    <span class="k-chip">${I('git-branch')}${o.branch || 'main'}${o.changed ? ` · ${o.changed} changed` : ''}</span>
    ${pr}
    ${o.server ? `<span class="k-chip"><span class="k-live"></span>${o.server}${I('arrow-up-right', 'k-ext')}</span>` : ''}
    <span class="k-sp"></span>
    ${btn('Commit', { i: 'git-commit', sc: '⇧⌘C' })}
    ${btn('Run agent', { k: 'pri', i: 'play', sc: '⇧⌘A' })}
    <span class="k-vline"></span>
    <span class="k-ptog">${tog}</span>
  </header>`;
}

/* ------------------------------------------------------------------ stage */
// tabs: [{a, t, st, on}]
const tabStrip = tabs => `<div class="k-tabs">${tabs.map(t => `<span class="k-tab ${t.on ? 'on' : ''}">${av(t.a || 'SH')}${t.t}${t.st ? si(t.st) : ''}${t.on ? I('x', 'k-x') : ''}</span>`).join('')}
  <span class="k-tadd">${I('plus')}</span><span class="k-sp"></span>${ib('square-split-horizontal', { s: true })}${ib('square-split-vertical', { s: true })}</div>`;
// o: {a, name, st, stl, task, place, timer, profile, lim}
const paneHead = o => `<div class="k-ph">${o.a ? av(o.a) : I('terminal-window')}<span class="k-nm">${o.name}</span>${o.st ? pill(o.st, o.stl) : ''}
  ${o.task ? `<span class="k-task">${o.task}</span>` : '<span class="k-sp"></span>'}
  ${o.place ? `<span class="k-place">${o.place}</span>` : ''}${o.timer ? `<span class="k-timer">${o.timer}</span>` : ''}
  ${o.profile ? `<span class="k-prof"><span class="k-dot ${o.lim ? 'lim' : ''}"></span>${o.profile}${I('caret-down', 'k-car')}</span>` : ''}
  <span class="k-pmenu">${I('dots-three')}</span></div>`;
const term = (lines, cls = '') => `<div class="k-term ${cls}">${lines.join('\n')}</div>`;
// o: head options + {focus, wait, notice, lines, style}
const pane = o => `<div class="k-pane ${o.focus ? 'focus' : ''} ${o.wait ? 'wait' : ''} ${o.notice ? 'notice' : ''}" style="${o.style || ''}">${paneHead(o)}${term(o.lines, o.tcls)}${o.notice || ''}</div>`;
const hsplit = (...kids) => `<div class="k-split">${kids.join('')}</div>`;
const vsplit = (...kids) => `<div class="k-split v">${kids.join('')}</div>`;

/* ------------------------------------------------------------------ status bar */
// o: {branch, ahead, lsp, waiting, running, mem}
function statusBar(o = {}) {
  const wait = o.waiting
    ? `<span class="k-g">${si('inp')}<span class="k-wait">${o.waiting} waiting</span>${kbd('⌘J')}</span>`
    : `<span class="k-g">${I('check')}Nothing waiting</span>`;
  return `<footer class="k-stb">
    <span class="k-g">${I('git-branch')}<span class="k-m k-t2">${o.branch || 'main'}</span><span class="k-m">${o.ahead || '↑2 ↓0'}</span></span>
    <span class="k-g"><span class="k-lsp"></span><span class="k-m">${o.lsp || 'rust-analyzer'}</span></span>
    <span class="k-g" style="gap:4px">${I('x-circle')}<span class="k-m">${o.errors ?? 0}</span>${I('warning')}<span class="k-m">${o.warnings ?? 2}</span></span>
    <span class="k-sp"></span>
    ${wait}
    ${o.running ? `<span class="k-g" style="gap:6px">${si('run')}<span class="k-t2">${o.running} running</span></span>` : ''}
    <span class="k-m">${o.mem || '312 MB'}</span>
  </footer>`;
}

/* ------------------------------------------------------------------ window */
// o: {theme, sb, bar, stage, status}
const win = o => `<div class="app ${o.theme || THEME()}"><div class="k-win">${o.sb}<div class="k-main">${o.bar}${o.stage}${o.status}</div></div></div>`;
const stage = (tabs, body) => `<section class="k-sheet k-stage">${tabStrip(tabs)}${body}</section>`;

/* ------------------------------------------------------------------ Accounts & usage popover */
// rows: [{a, id, plan, st, stl, p5, pw, reset}]
const usagePop = rows => `<div class="k-pop">
  <div style="display:flex;align-items:center"><h4>Accounts &amp; usage</h4><span class="k-sp"></span>${btn('Manage', { k: 'ghost', s: true })}</div>
  ${rows.map(r => `<div class="k-acc"><div class="k-a1">${av(r.a)}${r.id}<span class="k-plan">${r.plan}</span><span class="k-sp"></span>${pill(r.st, r.stl)}</div>
    <div class="k-meter"><span style="width:24px">5h</span>${track(r.p5, r.p5 >= 90 ? 'lim' : '')}<span class="k-v">${r.p5}%</span><span>${r.reset}</span></div>
    <div class="k-meter"><span style="width:24px">Wk</span>${track(r.pw)}<span class="k-v">${r.pw}%</span></div></div>`).join('')}
  <div style="display:flex;align-items:center;gap:8px;font-size:12px;color:var(--tx-2)">${I('lightning')}<span class="k-sp">Failover: switch automatically</span><span class="k-sw on"></span></div>
</div>`;

/* ------------------------------------------------------------------ the sample projects (design/figcraft data.py) */
const PROJECTS = [
  { n: 'storefront', letter: 'S', hue: 'blue' },
  { n: 'ternarycode', letter: 'T', hue: 'violet' },
  { n: 'api-gateway', letter: 'A', hue: 'green' },
  { n: 'mobile-app', letter: 'M', hue: 'rose' },
  { n: 'docs-site', letter: 'D', hue: 'amber' },
  { n: 'infra', letter: 'I', hue: 'cyan' },
];
const ps1 = (dir, br = 'main') => `<span class="b">~/code/${dir} </span><span class="m">${br} </span><span class="g">❯ </span>`;
const CUR = '<span class="cur"> </span>';
const TX = {
  vite: [
    `${ps1('storefront')}npm run dev`, '',
    `<span class="g">  VITE v6.0.3  </span><span class="d">ready in </span><span class="p">412</span><span class="d"> ms</span>`, '',
    `<span class="g">  ➜  </span><span class="p">Local:   </span><span class="c">http://localhost:5173/</span>`,
    `<span class="d">  ➜  Network: use --host to expose</span>`,
    `<span class="d">  ➜  press h + enter to show help</span>`, '',
  ],
  hmr: (t, f) => `<span class="d">${t} </span><span class="c">[vite] </span><span class="g">hmr update </span><span class="d">${f}</span>`,
  gitStatus: [
    `${ps1('storefront')}git status -sb`,
    `<span class="g">## main</span><span class="d">...origin/main [ahead 2]</span>`,
    `<span class="r"> M </span>src/auth/callback.ts`,
    `<span class="r"> M </span>src/auth/session.ts`,
    `<span class="r">?? </span>src/auth/returnTo.ts`,
    `${ps1('storefront')}${CUR}`,
  ],
  flutterFail: [
    `${ps1('mobile-app', 'dev')}flutter test`,
    `00:04 +31: loading test/cart_test.dart`,
    `<span class="r">00:05 +31 -1: Cart applies voucher [E]</span>`,
    `<span class="r">  Expected: &lt;90.0&gt;</span>`,
    `<span class="r">    Actual: &lt;100.0&gt;</span>`,
    `<span class="d">  test/cart_test.dart 42:7  main.&lt;fn&gt;</span>`, '',
    `<span class="r">00:06 +44 -1: Some tests failed.</span>`,
  ],
};
