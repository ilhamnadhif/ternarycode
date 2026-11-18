/* Lattice components (from design2/kit.js + screens/code.js): plain functions returning HTML,
   composed the way the app composes its screens from src/ui/kit.rs. */

const I = (n, cls = '') => { const cp = window.PH[n]; if (!cp) { console.warn('icon', n); return ''; } return `<i class="ph ${cls}">&#x${cp.toString(16)};</i>`; };
const IF = (n, cls = '') => { const cp = window.PH[n]; if (!cp) { console.warn('icon', n); return ''; } return `<i class="phf ${cls}">&#x${cp.toString(16)};</i>`; };
const esc = s => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const J = a => a.filter(Boolean).join('');

/* ------------------------------------------------------------------ data */
const PRJ = {
  'storefront':     { k: 'Ts', c: '#e07aa0', br: 'main', chg: 3, path: '~/code/storefront', kind: 'TypeScript · React' },
  'mobile-app':     { k: 'Fl', c: '#5aa6e6', br: 'feature/checkout', chg: 1, path: '~/code/mobile-app', kind: 'Flutter' },
  'ternary-engine': { k: 'Rs', c: '#e58a6b', br: 'main', chg: 2, path: '~/code/ternary-engine', kind: 'Rust' },
  'payments-api':   { k: 'Go', c: '#45b5a8', br: 'main', chg: 2, path: '~/code/payments-api', kind: 'Go' },
  'ml-pipeline':    { k: 'Py', c: '#9cbf5a', br: 'main', chg: 1, path: '~/code/ml-pipeline', kind: 'Python' },
  'docs-site':      { k: 'As', c: '#9d8cf0', br: 'main', chg: 0, path: '~/code/docs-site', kind: 'Astro' },
  'api-gateway':    { k: 'Go', c: '#8e9aab', br: 'main', chg: 0, path: '~/code/api-gateway', kind: 'Go' },
  'infra':          { k: 'Tf', c: '#c9a46a', br: 'main', chg: 0, path: '~/code/infra', kind: 'Terraform' },
};
const pt = (name, size = '') => { const p = PRJ[name] || { k: name.slice(0, 2), c: '#8e9aab' }; return `<span class="pt ${size}" style="--pc:${p.c}">${p.k}</span>`; };

/* ------------------------------------------------------------------ agent marks */
const LOGO = {
  CC: c => `<svg viewBox="0 0 24 24" width="${c}" height="${c}"><path fill="#D97757" d="m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z"/></svg>`,
  Cx: c => `<svg viewBox="0 0 24 24" width="${c}" height="${c}"><path fill="currentColor" d="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z"/></svg>`,
  Gm: c => `<svg viewBox="0 0 24 24" width="${c}" height="${c}"><path fill="#8E75B2" d="M11.04 19.32Q12 21.51 12 24q0-2.49.93-4.68.96-2.19 2.58-3.81t3.81-2.55Q21.51 12 24 12q-2.49 0-4.68-.93a12.3 12.3 0 0 1-3.81-2.58 12.3 12.3 0 0 1-2.58-3.81Q12 2.49 12 0q0 2.49-.96 4.68-.93 2.19-2.55 3.81a12.3 12.3 0 0 1-3.81 2.58Q2.49 12 0 12q2.49 0 4.68.96 2.19.93 3.81 2.55t2.55 3.81"/></svg>`,
  OC: c => `<svg viewBox="0 0 240 300" width="${Math.round(c * 0.8)}" height="${c}"><path fill="currentColor" d="M180 60H60V240H180V60ZM240 300H0V0H240V300Z"/><path fill="currentColor" fill-opacity=".28" d="M180 240H60V120H180V240Z"/></svg>`,
  AG: c => `<img src="assets/img/agents/antigravity.png" width="${c}" height="${c}" alt="">`,
  SH: c => `<i class="ph" style="font-size:${c}px;color:var(--text-3)">&#x${window.PH['terminal-window'].toString(16)};</i>`,
};
const AGN = { CC: 'Claude Code', Cx: 'Codex', OC: 'OpenCode', AG: 'Antigravity CLI', Gm: 'Gemini CLI', SH: 'zsh' };
const al = (a, size = 14, tile = 0) => tile
  ? `<span class="al tile" style="width:${tile}px;height:${tile}px;border-radius:${Math.round(tile * .27)}px">${LOGO[a](size)}</span>`
  : `<span class="al" style="width:${size}px;height:${size}px">${LOGO[a](size)}</span>`;

/* ------------------------------------------------------------------ status language */
// One state, one glyph, one colour, one word — in every place it shows.
const ST = {
  run:  { l: 'Running', c: 'run' },
  wait: { l: 'Needs input', c: 'wait', i: 'bell-ringing' },
  done: { l: 'Done', c: 'done', i: 'check-circle' },
  fail: { l: 'Failed', c: 'fail', i: 'x-circle' },
  idle: { l: 'Idle', c: 'idle' },
  act:  { l: 'Active', c: 'idle', i: 'circle-half' },
  cmd:  { l: 'Finished', c: 'done', i: 'check-circle' },
  res:  { l: 'Can resume', c: 'idle', i: 'clock-counter-clockwise' },
};
const lat = (st = 'run', size = '') => `<span class="lat ${st} ${size}" style="${st === 'run' ? '' : ''}">${'<b></b>'.repeat(9)}</span>`;
const gl = (st, sm = '') => {
  if (st === 'run') return `<span class="gl ${sm}">${lat('run')}</span>`;
  if (st === 'idle') return `<span class="gl ${sm}">${lat('idle')}</span>`;
  const s = ST[st];
  return `<span class="gl ${sm} c-${s.c}">${st === 'act' || st === 'res' ? I(s.i) : IF(s.i)}</span>`;
};
const pill = (st, label, cls = '') => !ST[st] ? `<span class="pill ${st} ${cls}">${label ?? ''}</span>` : `<span class="pill ${ST[st].c === 'idle' ? '' : ST[st].c} ${cls}">${st === 'run' ? lat('run') : st === 'idle' ? '' : (st === 'act' || st === 'res' ? I(ST[st].i) : IF(ST[st].i))}${label ?? ST[st].l}</span>`;
const stw = (st, label) => `<span class="st c-${ST[st].c}">${gl(st, 'sm')}${label ?? ST[st].l}</span>`;

/* ------------------------------------------------------------------ primitives */
const kbd = (k, cls = '') => `<span class="kbd ${cls}">${k}</span>`;
const btn = (label, o = {}) => `<span class="btn ${o.k || 'secondary'} ${o.sm ? 'sm' : ''} ${o.lg ? 'lg' : ''} ${o.cls || ''}">${o.i ? I(o.i) : ''}${o.al ? al(o.al, 14) : ''}${label}${o.kb ? `<span class="kb">${o.kb}</span>` : ''}</span>`;
const ib = (n, o = {}) => `<span class="ibtn ${o.sm ? 'sm' : ''} ${o.xs ? 'xs' : ''} ${o.on ? 'on' : ''} ${o.hov ? 'hov' : ''} ${o.acc ? 'acc' : ''} ${o.cls || ''}">${o.f ? IF(n) : I(n)}${o.dot ? '<b class="dot"></b>' : ''}</span>`;
const seg = (items, on, cls = '') => `<span class="seg ${cls}">${items.map((t, i) => `<span class="${i === on ? 'on' : ''}">${t}</span>`).join('')}</span>`;
const sw = on => `<span class="sw ${on ? 'on' : ''}"></span>`;
const chk = on => `<span class="chk ${on ? 'on' : ''}">${on ? I('check') : ''}</span>`;
const radio = on => `<span class="radio ${on ? 'on' : ''}"></span>`;
const vsep = () => '<span class="vsep"></span>';
const fld = (text, o = {}) => `<span class="fld ${o.focus ? 'focus' : ''} ${o.mono ? 'mono' : ''} ${o.cls || ''}" style="${o.w ? `width:${o.w}px;` : ''}${o.style || ''}">${o.i ? I(o.i) : ''}${o.ph ? `<span class="ph-t">${text}</span>` : `<span class="ell">${text}</span>`}${o.focus && !o.nocaret ? '<span class="caret"></span>' : ''}<span class="sp"></span>${o.right || ''}</span>`;
const sel = (text, w) => `<span class="sel" style="${w ? `width:${w}px` : ''}"><span class="ell">${text}</span><span class="sp"></span>${I('caret-up-down') || I('caret-down')}</span>`;
const bars = (a, d) => { const t = a + d || 1, n = 5, na = Math.round(a / t * n), nd = Math.min(n - na, Math.round(d / t * n)); return `<span class="bars">${'<b class="a"></b>'.repeat(na)}${'<b class="d"></b>'.repeat(nd)}${'<b></b>'.repeat(n - na - nd)}</span>`; };
const pos = (x, y, extra = '') => `left:${x}px;top:${y}px;${extra}`;

/* ------------------------------------------------------------------ sidebar */
// q: queue items {p, a, what, age, st, next}; groups: [{name, sum, rows:[...] }]
function sidebar(o = {}) {
  const view = o.view || 'focus';
  const q = o.queue ?? QUEUE;
  const queue = q.length ? `
    <div class="sb-h">Waiting for you <span class="badge wait" style="margin-left:2px">${q.length}</span><span class="sp"></span>${kbd('⌥⌘J')}</div>
    <div class="queue">${q.map((it, i) => `
      <div class="qi ${it.next ? 'next' : ''} ${it.here ? 'here' : ''} ${it.hov ? 'hov' : ''}">
        ${gl(it.st)}
        <span class="ti ell">${it.p}</span>
        <span class="age">${it.here ? '<span class="here-t">here</span>' : ''}${it.next ? kbd('⌘J') : ''}${it.age}</span>
        <span class="su">${al(it.a, 12)}<span class="ell">${it.what}</span></span>
      </div>`).join('')}
    </div>` : '';
  const groups = (o.groups ?? GROUPS(o.sel ?? 'storefront')).map(g => `
    ${g.name ? `<div class="grp">${I(g.open === false ? 'caret-right' : 'caret-down')}<span>${g.name}</span><span class="t4 mono" style="font-size:10.5px">${g.n ?? g.rows.length}</span>${g.sum ? `<span class="sum">${g.sum}</span>` : ''}</div>` : ''}
    ${g.open === false ? '' : g.rows.map(prow).join('')}`).join('');
  return `<aside class="sb">
    <div class="sb-top">${o.nolights ? '' : '<span class="lights"><b></b><b></b><b></b></span>'}<span class="sp"></span>${ib('sidebar-simple', { sm: true })}</div>
    <div style="padding:2px 0 10px">${seg([`${I('terminal-window')}Focus`, `${I('squares-four')}Grid`], view === 'grid' ? 1 : 0, 'full')}</div>
    <div class="sb-scroll">
      ${queue}
      <div class="sb-h" style="margin-top:${q.length ? 4 : 0}px">Projects <span class="n">${o.count ?? 8}</span><span class="sp"></span>${ib('magnifying-glass', { sm: true })}${ib('plus', { sm: true })}</div>
      ${o.search ? `<div style="padding:0 0 6px">${o.search}</div>` : ''}
      ${groups}
    </div>
    <div class="sb-foot">${I('gear-six')}<span>Settings</span><span class="sp"></span><span class="mono t4" style="font-size:10.5px">⌘,</span></div>
  </aside>`;
}
// row: {n, st, stl, a, badge:[cls,n], dots, sel, hov, dim, child}
function prow(r) {
  const p = PRJ[r.n] || {};
  const st = r.st ? `<span class="st c-${ST[r.st].c}">${r.a && r.st === 'run' ? al(r.a, 11) : gl(r.st, 'sm')}<span class="ell">${r.stl ?? ST[r.st].l}</span></span>` : '';
  const chg = r.chg ?? p.chg;
  return `<div class="pr ${r.sel ? 'on' : ''} ${r.hov ? 'hov' : ''} ${r.dim ? 'dim' : ''} ${r.child ? 'child' : ''}">
    ${r.child ? `<span class="pt wt s16">${I('git-fork')}</span>` : pt(r.n)}
    <div class="pr-b">
      <div class="l1"><span class="nm ell">${r.label || r.n}</span>${r.dots ? `<span class="pdots">${r.dots.map(d => d === 'web' ? I('globe') : `<b class="${d}"></b>`).join('')}</span>` : ''}<span class="sp"></span>${r.badge ? `<span class="badge ${r.badge[0]}">${r.badge[1]}</span>` : ''}${r.r1 || ''}</div>
      <div class="l2"><span class="meta">${I(r.child ? 'git-branch' : 'git-branch')}<span class="ell">${r.br || p.br}</span>${chg ? `<span class="t2">+${chg}</span>` : ''}</span><span class="sp"></span>${st}</div>
    </div>
  </div>`;
}
const QUEUE = [
  { p: 'mobile-app', a: 'CC', what: 'Bash · flutter test', age: '2m', st: 'wait', next: true },
  { p: 'ternary-engine', a: 'CC', what: 'Bash · cargo test --workspace', age: '41s', st: 'wait' },
  { p: 'payments-api', a: 'Cx', what: 'Finished · 2 files changed', age: '3m', st: 'done' },
];
const SUM = (r = 3, w = 2, d = 1) => `<span class="c-run">${lat('run')}${r}</span><span class="c-wait">${IF('bell-ringing')}${w}</span><span class="c-done">${IF('check-circle')}${d}</span>`;
const GROUPS = (selName) => [
  { name: 'Active', n: 6, sum: SUM(), rows: [
    { n: 'docs-site', st: 'run', a: 'SH', stl: 'npm run dev', dots: ['r'] },
    { n: 'ml-pipeline', st: 'run', a: 'Cx', stl: 'Codex · 62%', dots: ['r'] },
    { n: 'mobile-app', st: 'wait', badge: ['wait', 1], dots: ['w', ''] },
    { n: 'payments-api', st: 'done', badge: ['done', 1], dots: ['d'] },
    { n: 'storefront', st: 'run', a: 'CC', stl: 'Claude Code', dots: ['r', 'r', 'web'] },
    { n: 'ternary-engine', st: 'wait', badge: ['wait', 1], dots: ['w'] },
  ] },
  { name: 'Idle', n: 2, rows: [
    { n: 'api-gateway' },
    { n: 'infra' },
  ] },
].map(g => ({ ...g, rows: g.rows.map(r => ({ ...r, sel: r.n === selName })) }));

/* ------------------------------------------------------------------ top bar */
function topbar(o = {}) {
  const n = o.p || 'storefront', p = PRJ[n] || {};
  const tabs = (o.tabs || [{ g: 'run', l: 'Claude Code', on: true }, { g: 'run', l: 'node', i: 'terminal-window' }]).map(t =>
    `<span class="tab ${t.on ? 'on' : ''}">${t.al ? al(t.al, 13) : t.g ? gl(t.g, 'sm') : I(t.i || 'terminal-window')}${t.l}${t.w ? '<span class="wdot"></span>' : ''}${t.prog ? `<span class="prog"><i style="width:${t.prog}%"></i></span>` : ''}<span class="x">${I('x')}</span></span>`).join('');
  return `<header class="tb">
    ${o.left ?? `<span class="ttl">${pt(n, 's20')}<span class="nm">${o.title || n}</span>${I('caret-down', 'chev')}</span>
    <span class="chip mono">${I('git-branch')}${o.br || p.br}</span>
    ${(o.chg ?? p.chg) ? `<span class="chip ${o.chgHov ? 'hov' : ''}">${I('git-diff')}<span>${o.chg ?? p.chg} changed</span></span>` : ''}
    ${o.server === false ? '' : `<span class="chip mono"><span class="sdot"></span>${o.server || 'localhost:5173'}${I('arrow-up-right', 'ext')}</span>`}
    ${vsep()}
    <div class="tabs">${tabs}${ib('plus', { sm: true })}</div>`}
    <span class="sp"></span>
    ${o.right ?? `${btn('Run Agent', { k: 'primary', i: 'play', kb: '⇧⌘A', sm: false })}
    <span class="vsep"></span>
    <span class="igrp">${ib('square-split-horizontal')}${ib('square-split-vertical')}</span>
    <span class="vsep"></span>
    <span class="igrp">${ib('globe', { on: o.web, dot: o.webDot })}${ib('code', { on: o.code })}</span>`}
  </header>`;
}

/* ------------------------------------------------------------------ status bar */
function statusbar(o = {}) {
  const lead = o.lead ?? `<span class="lead">${IF('bell-ringing', 'c-wait')}<b>mobile-app</b><span class="t3">·</span>Claude Code wants to run <span class="mono" style="font-size:10.5px;color:var(--text)">flutter test</span><span class="t3">+2 more</span>${kbd('⌘J')}</span>`;
  return `<footer class="stb">${lead}<span class="sp"></span>
    ${o.extra || ''}
    <span class="m">${o.stats || '8 projects · 9 panes · 141 MB · pages 44 MB'}</span>
    <span class="t4">·</span>
    <span class="row g6 t2">${I('magnifying-glass')}Commands ${kbd('⌘K')}</span>
  </footer>`;
}

/* ------------------------------------------------------------------ sheets & panes */
const sheet = (inner, o = {}) => `<section class="sheet ${o.cls || ''}" style="${o.style || ''}">${inner}</section>`;
function paneHead(o = {}) {
  const a = o.a;
  const mark = a && a !== 'SH' ? al(a, 14) : `<span class="gl sm t3">${I(o.icon || 'terminal-window')}</span>`;
  return `<div class="ph-h">
    ${mark}
    <span class="nm">${o.name || AGN[a] || 'zsh'}</span>
    ${o.st && o.st !== 'idle' && o.stl !== '' ? `<span class="st c-${ST[o.st].c}">${gl(o.st, 'sm')}${o.stl ?? ST[o.st].l}</span>` : o.st === 'run' ? gl('run', 'sm') : ''}
    ${o.task ? `<span class="task ell">${o.task}</span>` : ''}
    <span class="sp"></span>
    ${o.right || ''}
    ${o.meta ? `<span class="mt">${o.meta}</span>` : ''}
    ${o.hooks ? `<span class="mt row g4" title="Status from the agent's hooks">${I('plugs-connected')}hooks</span>` : ''}
    ${o.time ? `<span class="mt" style="color:var(--text-2)">${o.time}</span>` : ''}
    ${o.nomenu ? '' : ib('dots-three', { xs: true })}
  </div>`;
}
const term = (lines, cls = '') => `<div class="term ${cls}">${Array.isArray(lines) ? lines.join('\n') : lines}</div>`;
const pane = (o, lines) => sheet(paneHead(o) + term(lines, o.tcls) + (o.foot || ''), { cls: o.cls, style: o.style });

/* ------------------------------------------------------------------ window */
function win(o) {
  return `<div class="app ${o.theme || ''}"><div class="win ${o.cls || ''} ${o.sb === false ? 'nosb' : ''}" style="${o.style || ''}">
    ${o.sb === false ? '' : (o.sb || sidebar())}
    ${o.top ?? topbar()}
    <main class="main" style="${o.mainStyle || ''}">${o.main}</main>
    ${o.status ?? statusbar()}
    ${o.over || ''}
  </div></div>`;
}

/* ------------------------------------------------------------------ terminal scenes */
const vis = s => s.replace(/<[^>]+>/g, '').replace(/&(lt|gt|amp|nbsp);/g, 'x').length;
const CUR = '<span class="cur"> </span>', CURH = '<span class="cur hollow"> </span>';
// A box drawn with characters, as agents draw them: rows are padded to the inner width.
const boxed = (rows, w, cls = 'box') => [
  `<span class="${cls}">╭${'─'.repeat(w + 2)}╮</span>`,
  ...rows.map(r => `<span class="${cls}">│</span> ${r}${' '.repeat(Math.max(0, w - vis(r)))} <span class="${cls}">│</span>`),
  `<span class="${cls}">╰${'─'.repeat(w + 2)}╯</span>`,
];
const T = {
  ccHead: (model = 'opus 4.6', dir = '~/code/storefront') => [
    `<span class="cc">✻</span> <span class="b">Claude Code</span> <span class="d">· ${model} · ${dir}</span>`, '',
  ],
  storefrontAgent: [
    `<span class="d">&gt;</span> Add a cart badge to the header, then check it in the browser`,
    '',
    `<span class="g">⏺</span> Read <span class="b">src/components/Header.tsx</span>`,
    `  <span class="f">⎿</span>  <span class="d">84 lines</span>`,
    `<span class="g">⏺</span> Write <span class="b">src/components/CartBadge.tsx</span> <span class="g">+31</span>`,
    `<span class="g">⏺</span> Update <span class="b">src/components/Header.tsx</span> <span class="g">+4</span> <span class="r">−1</span>`,
    `<span class="g">⏺</span> Update <span class="b">src/styles/header.css</span> <span class="g">+12</span>`,
    `<span class="g">⏺</span> <span class="b">ternarycode</span> <span class="d">·</span> browser_navigate <span class="d">localhost:5173</span>`,
    `<span class="g">⏺</span> <span class="b">ternarycode</span> <span class="d">·</span> browser_click <span class="d">“Add to cart” on Ridge 28 Daypack</span>`,
    `  <span class="f">⎿</span>  <span class="d">The badge shows 1. Clicking a second item…</span>`,
    '',
    `<span class="u">✻</span> <span class="u">Checking the count goes up</span> <span class="d">(14s · ↑ 2.1k tokens · esc to interrupt)</span>`,
  ],
  vite: [
    `<span class="m">storefront</span> <span class="d">on</span> <span class="m"> main</span> <span class="g">❯</span> npm run dev`,
    '',
    `<span class="d">&gt;</span> storefront@0.4.0 dev`,
    `<span class="d">&gt;</span> vite`,
    '',
    `  <span class="g b">VITE</span> <span class="g">v6.0.3</span>  <span class="d">ready in</span> <span class="b">312</span> <span class="d">ms</span>`,
    '',
    `  <span class="g">➜</span>  <span class="b">Local</span>:   <span class="c">http://localhost:<span class="b">5173</span>/</span>`,
    `  <span class="g">➜</span>  <span class="d">Network: use</span> <span class="b">--host</span> <span class="d">to expose</span>`,
    `  <span class="d">10:41:07</span> <span class="c">[vite]</span> <span class="g">hmr update</span> <span class="d">/src/components/Header.tsx</span>`,
    `  <span class="d">10:41:09</span> <span class="c">[vite]</span> <span class="g">hmr update</span> <span class="d">/src/styles/header.css</span>`,
  ],
  flutterAsk: [
    `<span class="d">&gt;</span> The checkout total is a cent off. Fix it and run the tests`,
    '',
    `<span class="g">⏺</span> Read <span class="b">lib/checkout/cart_total.dart</span>`,
    `  It rounds before the discount; it should round once, at the end.`,
    `<span class="g">⏺</span> Update <span class="b">lib/checkout/cart_total.dart</span> <span class="g">+3</span> <span class="r">−2</span>`,
    '',
    ...boxed([
      `<span class="b">Bash command</span>`, '',
      `  flutter test test/checkout_test.dart`,
      `  <span class="d">Run the checkout tests</span>`, '',
      `Do you want to proceed?`,
      `<span class="u">❯ 1. Yes</span>`,
      `  2. Yes, and don’t ask again for <span class="b">flutter test</span>`,
      `  3. No, and tell Claude what to do differently`,
    ], 50),
  ],
  cargoAsk: [
    `<span class="d">&gt;</span> Make the parser handle nested generics and add tests`,
    '',
    `<span class="g">⏺</span> Read <span class="b">src/parser.rs</span> <span class="d">(412 lines)</span>`,
    `  <span class="b">Vec&lt;Option&lt;T&gt;&gt;</span> ends early: the parser closes on`,
    `  the first <span class="b">&gt;</span>. I’ll track the depth and split <span class="b">&gt;&gt;</span>.`,
    `<span class="g">⏺</span> Update <span class="b">src/parser.rs</span> <span class="g">+38</span> <span class="r">−9</span>`,
    `<span class="g">⏺</span> Write <span class="b">tests/generics.rs</span> <span class="g">+64</span>`,
    '',
    `<span class="y">?</span> Run <span class="b">cargo test --workspace</span>?`,
    `  <span class="u">❯ Yes</span>   Always   No`,
  ],
  codexDone: [
    `<span class="b">&gt;_ codex</span> <span class="d">· gpt-5.5 · ~/code/payments-api</span>`, '',
    `<span class="d">&gt;</span> Add idempotency keys to POST /charges`,
    '',
    `<span class="g">•</span> Edited <span class="b">charges/handler.go</span> <span class="g">+41</span> <span class="r">−6</span>`,
    `<span class="g">•</span> Added <span class="b">charges/idempotency.go</span> <span class="g">+73</span>`,
    `<span class="g">•</span> Ran <span class="b">go test ./...</span> <span class="g">ok</span> <span class="d">(2.4s)</span>`,
    '',
    `  Repeated requests with the same Idempotency-Key`,
    `  now get the first response back.`,
  ],
  codexTrain: [
    `<span class="b">&gt;_ codex</span> <span class="d">· gpt-5.5 · ~/code/ml-pipeline</span>`, '',
    `<span class="d">&gt;</span> Add the hour feature and retrain`,
    '',
    `<span class="g">•</span> Edited <span class="b">pipeline/features.py</span> <span class="g">+3</span> <span class="r">−1</span>`,
    `<span class="y">•</span> Running <span class="b">python -m pipeline.train</span>`,
    `  <span class="d">[400/600]</span> auc <span class="b">0.912</span>  logloss 0.231`,
    `  <span class="d">[500/600]</span> auc <span class="b">0.915</span>  logloss 0.228`,
  ],
  astro: [
    `<span class="m">docs-site</span> <span class="g">❯</span> npm run dev`,
    '',
    ` <span class="g b">astro</span>  <span class="g">v5.0.3</span> <span class="d">ready in</span> 418 <span class="d">ms</span>`,
    '',
    ` ┃ Local    <span class="c">http://localhost:4321/</span>`,
    ` ┃ Network  <span class="d">use --host to expose</span>`,
    '',
    `<span class="d">10:39:52</span> <span class="c">[watch]</span> docs/checkout.md`,
  ],
  idle: (n, br = 'main') => [`<span class="m">${n}</span> <span class="d">on</span> <span class="m"> ${br}</span> <span class="g">❯</span> ${CUR}`],
  prompt: (w = 58) => boxed([`<span class="d">&gt;</span> ${CURH}`], w),
};


/* ------------------------------------------------------------------ browser panel */
// pages: [{fav|a, t, on, dot, snd}], url, agent: {a, doing} | null, page: html
function browserPanel(o = {}) {
  const pages = (o.pages || [{ a: 'CC', t: 'Fernway — Trail gear', on: true, dot: true }, { fav: 'book-open', t: 'Docs · Checkout' }]).map(p =>
    `<span class="bp-tab ${p.on ? 'on' : ''}">${p.a ? al(p.a, 12) : I(p.fav || 'globe', 't3')}<span class="ell">${p.t}</span>${p.dot ? '<b class="bdot"></b>' : ''}${p.snd ? I('speaker-high', 't3') : ''}${p.on ? `<span class="x">${I('x')}</span>` : ''}</span>`).join('');
  const ag = o.agent;
  return sheet(`
    <div class="bp-tabs">${pages}${ib('plus', { xs: true })}<span class="sp"></span>${ib('arrows-out', { xs: true })}${ib('x', { xs: true })}</div>
    <div class="bp-bar">
      <span class="igrp">${ib('arrow-left', { sm: true })}${ib('arrow-right', { sm: true, cls: 'dis' })}${ib('arrow-clockwise', { sm: true })}</span>
      <span class="bp-url">${I(o.lock ? 'lock-simple' : 'globe-simple', 't3')}<span class="mono">${o.url || 'localhost:5173'}</span><span class="mono t4">${o.path || '/'}</span><span class="sp"></span>${o.zoom ? `<span class="mono t3" style="font-size:10.5px">${o.zoom}</span>` : ''}</span>
      ${o.console ? `<span class="chip mono" style="height:24px;padding:0 7px;color:var(--wait)">${I('warning', 'c-wait')}${o.console}</span>` : ''}
      ${ib('tray-arrow-down', { sm: true })}${ib('dots-three', { sm: true })}
    </div>
    ${ag ? `<div class="bp-agent">${lat('run')}${al(ag.a, 13)}<span><b>${AGN[ag.a]}</b> is using this page</span><span class="t3">·</span><span class="ell t2">${ag.doing}</span><span class="sp"></span>${btn('Take Over', { sm: true, i: 'hand-pointing' })}</div>` : ''}
    <div class="bp-page ${ag ? 'agent' : ''}">${o.page || ''}</div>
  `, { cls: (o.cls || '') + ' bp', style: o.style });
}

/* the demo shop the agents work on */
function fernway(o = {}) {
  const items = [
    { n: 'Ridge 28 Daypack', d: '28 L · 840 g', p: '$129', i: 'backpack', bg: '#e8efe6', fg: '#3f6b46', added: true },
    { n: 'Saddle UL Tent', d: '2 person · 1.3 kg', p: '$389', i: 'tent', bg: '#f1ebe2', fg: '#8a5a2b' },
    { n: 'Scree Mid Boot', d: 'Waterproof · 920 g', p: '$175', i: 'boot', bg: '#e6ebf1', fg: '#3d5a7a' },
    { n: 'Bearing Compass', d: 'Baseplate · 38 g', p: '$39', i: 'compass', bg: '#efe7ee', fg: '#7a4a6d' },
  ];
  return `<div class="fw">
    <div class="fw-h"><span class="fw-logo"><b></b>Fernway</span><span class="fw-nav"><span>Packs</span><span>Shelter</span><span>Footwear</span><span>Journal</span></span><span class="sp"></span><span class="fw-cart">${I('bag-simple') || I('handbag')}<b>${o.count ?? 1}</b></span></div>
    <div class="fw-hero"><div><div class="fw-k">Autumn 2026</div><h2>Ready for the trail</h2><p>Light packs and shelters, tested on long weekends.</p></div></div>
    <div class="fw-sec"><b>New this season</b><span>12 products</span></div>
    <div class="fw-grid">${items.map((it, k) => `
      <div class="fw-card"><div class="fw-img" style="background:${it.bg};color:${it.fg}">${I(it.i)}</div>
        <div class="fw-b"><div class="fw-n">${it.n}</div><div class="fw-d">${it.d}</div>
        <div class="fw-r"><b>${it.p}</b><span class="fw-btn ${(o.added ? o.added.includes(k) : it.added) ? 'added' : ''} ${o.hl === k ? 'hl' : ''}">${(o.added ? o.added.includes(k) : it.added) ? 'Added' : 'Add to cart'}</span></div></div></div>`).join('')}
    </div>
    ${o.cursor ? `<span class="acur" style="${o.cursor}">${I('cursor', 'c')}</span>` : ''}
  </div>`;
}

/* ------------------------------------------------------------------ code & review (screens/code.js) */
/* ------------------------------------------------------------------ syntax */
const CD_KW = {
  rs: 'fn|let|mut|if|else|while|loop|match|return|pub|struct|enum|impl|use|self|Self|for|in|break|continue|as|ref|const|mod|crate|where|true|false|dyn|move|trait|type',
  ts: 'import|from|export|default|function|const|let|return|if|else|type|interface|extends|new|await|async|true|false|null|undefined|as|of|in|for',
  go: 'func|package|import|return|if|else|for|range|var|const|type|struct|interface|nil|true|false|err|defer|go|map',
  css: '',
};
function cdHl(src, lang = 'rs') {
  const parts = [
    '(\\/\\/.*$)',
    lang === 'rs' ? '("(?:[^"\\\\]|\\\\.)*")' : '("(?:[^"\\\\]|\\\\.)*"|\'(?:[^\'\\\\]|\\\\.)*\'|`[^`]*`)',
    lang === 'ts' ? '(<\\/?[A-Za-z][\\w.]*|\\/?>)' : '(\\u0000)',
    '(\\b\\d+(?:\\.\\d+)?(?:ms|px|rem)?\\b)',
    '(\\b[a-z_][a-z0-9_]*!)',
    CD_KW[lang] ? `(\\b(?:${CD_KW[lang]})\\b)` : '(\\u0000)',
    '(\\b[A-Z][A-Za-z0-9_]*\\b)',
    '(\\b[a-zA-Z_][a-zA-Z0-9_]*(?=\\())',
  ];
  const cls = [null, 'com', 'str', 'ty', 'num', 'mac', 'kw', 'ty', 'fn'];
  const re = new RegExp(parts.join('|'), 'g');
  let out = '', last = 0, m;
  while ((m = re.exec(src))) {
    if (m[0] === '') { re.lastIndex++; continue; }
    out += esc(src.slice(last, m.index));
    const k = m.findIndex((g, i) => i > 0 && g !== undefined);
    out += `<span class="${cls[k]}">${esc(m[0])}</span>`;
    last = re.lastIndex;
  }
  return out + esc(src.slice(last));
}
// ⟦…⟧ marks a word-level change inside a diff line.
const cdHlw = (src, lang, wcls) => src.split(/⟦|⟧/).map((s, i) => i % 2 ? `<span class="${wcls}">${cdHl(s, lang)}</span>` : cdHl(s, lang)).join('');

/* ------------------------------------------------------------------ drawer parts */
const cdMark = m => m ? `<span class="cd-m ${m}">${m === 'a' ? 'A' : m === 'd' ? 'D' : 'M'}</span>` : '';
// rows: [depth, kind('dir'|'file'), name, {m, open, on, ag, icon}]
function cdTree(rows) {
  return rows.map(([d, k, n, o = {}]) => `
    <div class="cd-tr ${o.on ? 'on' : ''} ${o.m ? 'chg' : ''}" style="padding-left:${8 + d * 14}px">
      ${k === 'dir' ? I(o.open ? 'caret-down' : 'caret-right', 'cd-car') : '<span class="cd-car"></span>'}
      ${k === 'dir' ? IF(o.open ? 'folder-open' : 'folder', 'cd-fi dir') : I(o.icon || 'file-code', 'cd-fi')}
      <span class="ell">${n}</span>${o.dot ? '<b class="cd-tdot"></b>' : ''}
      <span class="sp"></span>${o.ag ? al(o.ag, 11) : ''}${cdMark(o.m)}
    </div>`).join('');
}
const cdHead = (tab, o = {}) => `<div class="cd-dh">
  ${seg([`${I('files')}Files`, `${I('git-diff')}Changes <span class="n">${o.n ?? 5}</span>`], tab)}
  <span class="sp"></span>${o.extra || ''}${ib('magnifying-glass', { sm: true })}${ib('arrows-out', { sm: true })}${ib('x', { sm: true })}
</div>`;
// lines: [n, text, gutterMark('a'|'m'|'d'|'')]
function cdEditor(lines, lang, o = {}) {
  return `<div class="cd-ed code">${lines.map(([n, t, g]) => `<div class="cd-l ${n === o.cur ? 'cur' : ''}"><span class="cd-g ${g || ''}"></span><span class="ln">${n}</span><span class="cd-t">${o.raw ? t : cdHl(t, lang)}</span></div>`).join('')}${o.over || ''}</div>`;
}
const cdDl = (type, a, b, text, lang = 'rs') => {
  const sign = type === 'add' ? '+' : type === 'del' ? '−' : '';
  return `<div class="dl ${type}"><span class="a">${a}</span><span class="b">${b}</span><span class="s">${sign}</span><span class="t">${cdHlw(text, lang, type === 'add' ? 'wa' : 'wd')}</span></div>`;
};
const cdFiles = (rows) => rows.map(r => `
  <div class="cd-fr ${r.on ? 'on' : ''}">
    ${cdMark(r.m)}<div style="min-width:0;flex:1"><div class="ell"><span class="t1">${r.n}</span></div><div class="mono t3 ell" style="font-size:10.5px;margin-top:1px">${r.d}</div></div>
    ${r.ag ? al(r.ag, 11) : ''}
    <span class="cd-pm"><span class="c-add">+${r.a}</span>${r.x ? `<span class="c-del">−${r.x}</span>` : ''}</span>
  </div>`).join('');
/* ------------------------------------------------------------------ 3. full review, split, worktree */
// [type, leftNo, leftText, rightNo, rightText]
const HEADER_SPLIT = [
  ['', 1, 'import { Link } from "react-router-dom";', 1, 'import { Link } from "react-router-dom";'],
  ['', 2, 'import { Logo } from "./Logo";', 2, 'import { Logo } from "./Logo";'],
  ['', 3, 'import { Nav } from "./Nav";', 3, 'import { Nav } from "./Nav";'],
  ['', 4, 'import { CartButton } from "./CartButton";', 4, 'import { CartButton } from "./CartButton";'],
  ['add', '', '', 5, 'import { CartBadge } from "./CartBadge";'],
  ['add', '', '', 6, 'import { useCart } from "../cart/useCart";'],
  ['', 5, '', 7, ''],
  ['', 6, 'export function Header() {', 8, 'export function Header() {'],
  ['add', '', '', 9, '  const { count } = useCart();'],
  ['', 7, '  return (', 10, '  return ('],
  ['', 8, '    <header className="site-header">', 11, '    <header className="site-header">'],
  ['', 9, '      <Logo />', 12, '      <Logo />'],
  ['', 10, '      <Nav />', 13, '      <Nav />'],
  ['mod', 11, '      <CartButton⟦ /⟧>', 14, '      <CartButton>'],
  ['add', '', '', 15, '        <CartBadge count={count} />'],
  ['add', '', '', 16, '      </CartButton>'],
  ['', 12, '    </header>', 17, '    </header>'],
  ['', 13, '  );', 18, '  );'],
  ['', 14, '}', 19, '}'],
];
function cdSplit(rows, lang) {
  const side = (t, n, text, right) => {
    const empty = n === '';
    const k = t === 'add' ? (right ? 'add' : 'gap') : t === 'del' ? (right ? 'gap' : 'del') : t === 'mod' ? (right ? 'add' : 'del') : '';
    return `<div class="dl ${k === 'gap' ? 'cd-gap' : k}"><span class="a">${n}</span><span class="s">${k === 'add' ? '+' : k === 'del' ? '−' : ''}</span><span class="t">${empty ? '' : cdHlw(text, lang, k === 'add' ? 'wa' : 'wd')}</span></div>`;
  };
  return `<div class="cd-split code">
    <div class="cd-sp">${rows.map(r => side(r[0], r[1], r[2], false)).join('')}</div>
    <div class="cd-sp">${rows.map(r => side(r[0], r[3], r[4], true)).join('')}</div>
  </div>`;
}
