// TernaryCode's helper for agents using a browser pane (`browser_agent.rs`): a text outline of
// the page whose elements carry refs, acting on those elements, the console's messages, and a
// ring that shows the user what the agent is doing. It lives on `window.__tc` (made once per
// page: at document start, and again by any call that finds it missing). Every function
// returns a plain object: `{error}` when it could not do its job.
if (!window.__tc) window.__tc = (() => {
  // ----- the console, from the start of the page: kept here for the agents' tool, and sent to
  // TernaryCode (its console drawer), in batches at most four times a second
  const log = [];
  let outbox = [];
  let sending = 0;
  // Where a message was written: the file and line of the caller (best effort).
  const source = () => {
    const lines = String(new Error().stack || '').split('\n');
    for (const l of lines.slice(3)) {
      const m = l.match(/([^/@\s()]+?)(?:\?[^:]*)?:(\d+):\d+\)?$/);
      if (m) return `${m[1]}:${m[2]}`;
    }
    return '';
  };
  const send = (level, text, src) => {
    outbox.push([level, text.slice(0, 2000), src || '']);
    if (outbox.length > 200) outbox.shift();
    if (!sending) {
      sending = setTimeout(() => {
        sending = 0;
        const batch = outbox;
        outbox = [];
        try { window.ipc.postMessage('console:' + JSON.stringify(batch)); } catch (e) { /* no app */ }
      }, 250);
    }
  };
  const say = (line, level, src) => {
    log.push(line.slice(0, 2000));
    if (log.length > 300) log.shift();
    if (level) send(level, line.replace(/^\[\w+\] /, ''), src);
  };
  const show = (a) => {
    if (typeof a === 'string') return a;
    if (a instanceof Error) return a.stack || String(a);
    try { return JSON.stringify(a); } catch (e) { return String(a); }
  };
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const original = console[level];
    console[level] = function (...args) {
      say(`[${level}] ${args.map(show).join(' ')}`, level, source());
      return original.apply(this, args);
    };
  }
  addEventListener('error', (e) => {
    const file = String(e.filename || '').split('/').pop().split('?')[0];
    say(`[error] ${e.message} (${e.filename || 'page'}:${e.lineno || 0})`, 'error', file ? `${file}:${e.lineno || 0}` : '');
  });
  addEventListener('unhandledrejection', (e) => say(`[error] Unhandled rejection: ${show(e.reason)}`, 'error', ''));

  // ----- what sleeping would end, told to TernaryCode (`state:`), which keeps such a page awake
  // and shows a speaker on its tab: sound playing (a: audio or video playing, not muted by the
  // page), the camera, microphone or screen in use (c), a form being filled in (e: text typed
  // in a field and not sent yet), and the user's mute from the tab (m), kept on new media.
  let muted = false;
  const played = new Set();
  const streams = new Set();
  const typed = new Set();
  // Each new page tells its state once (the page before it may have been playing).
  let sent = '';
  let due = 0;
  let polling = 0;
  const medias = () => {
    const all = new Set(played);
    try { for (const m of document.querySelectorAll('audio, video')) all.add(m); } catch (e) { /* no document yet */ }
    return all;
  };
  const sounding = (m) => !m.paused && !m.ended && m.volume > 0 && (!m.muted || m.__tcMuted === true);
  const searchy = (el) => el.type === 'search' || /^(searchbox|combobox)$/.test(el.getAttribute('role') || '') || /search|query|^q$/i.test(el.name || '') || !!el.closest('form[role="search"], [role="search"]');
  const filled = (el) => {
    if (!el.isConnected) return false;
    if (el.isContentEditable) return (el.textContent || '').trim() !== '';
    return String(el.value || '').trim() !== '' && el.value !== el.defaultValue && !searchy(el);
  };
  const live = () => {
    for (const s of streams) {
      if (s.getTracks().some((t) => t.readyState === 'live')) return true;
      streams.delete(s);
    }
    return false;
  };
  // Only the page itself tells (on Windows the helper runs in its frames too).
  const topFrame = (() => { try { return window.top === window; } catch (e) { return false; } })();
  const report = () => {
    due = 0;
    if (!topFrame) return;
    const capturing = live();
    for (const el of typed) if (!el.isConnected) typed.delete(el);
    const now = JSON.stringify({ a: [...medias()].some(sounding), c: capturing, e: [...typed].some(filled), m: muted });
    // A stream stopped with track.stop() says nothing: looked at again while one is live.
    if (capturing && !polling) polling = setInterval(report, 2000);
    if (!capturing && polling) { clearInterval(polling); polling = 0; }
    if (now === sent) return;
    sent = now;
    try { window.ipc.postMessage('state:' + now); } catch (e) { /* no app */ }
  };
  const soon = (ms) => { if (!due) due = setTimeout(report, ms); };
  const quiet = (m) => { if (muted && !m.muted) { m.muted = true; m.__tcMuted = true; } };
  const seen = (m) => { if (m instanceof HTMLMediaElement) { played.add(m); quiet(m); } };
  for (const type of ['play', 'playing', 'pause', 'ended', 'emptied', 'volumechange']) {
    addEventListener(type, (e) => {
      const m = e.target;
      if (!(m instanceof HTMLMediaElement)) return;
      if (type === 'play') seen(m);
      // The page unmuted it (its own button): no longer the tab's mute.
      if (type === 'volumechange' && !m.muted) m.__tcMuted = false;
      soon(50);
    }, true);
  }
  // Media made by scripts and never put in the page (new Audio()) is heard of when it plays.
  try {
    const play = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function (...args) {
      seen(this);
      const r = play.apply(this, args);
      soon(50);
      return r;
    };
  } catch (e) { /* kept as it is */ }
  try {
    const md = navigator.mediaDevices;
    for (const name of ['getUserMedia', 'getDisplayMedia']) {
      const f = md && md[name];
      if (typeof f !== 'function') continue;
      md[name] = function (...args) {
        return f.apply(this, args).then((s) => {
          streams.add(s);
          for (const t of s.getTracks()) t.addEventListener('ended', () => soon(50));
          soon(50);
          return s;
        });
      };
    }
  } catch (e) { /* kept as it is */ }
  addEventListener('input', (e) => {
    const el = e.composedPath ? e.composedPath()[0] : e.target;
    if (el && (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el.isContentEditable)) {
      if (el instanceof HTMLInputElement && /^(checkbox|radio|range|color|file|submit|button|hidden)$/.test(el.type)) return;
      typed.add(el);
      soon(400);
    }
  }, true);
  // Sent: Enter in a one-line field, or the form's submit.
  addEventListener('keydown', (e) => {
    const el = e.composedPath ? e.composedPath()[0] : e.target;
    if (e.key === 'Enter' && el instanceof HTMLInputElement) { typed.delete(el); soon(400); }
  }, true);
  addEventListener('submit', (e) => {
    for (const el of typed) if (e.target.contains(el)) typed.delete(el);
    soon(400);
  }, true);
  // Told once the page is there (not from the blank page a new window starts with: WebView2 is
  // still making that window then).
  const first = () => { if (location.href !== 'about:blank') soon(0); };
  if (document.readyState === 'loading') addEventListener('DOMContentLoaded', first, { once: true });
  else first();
  const mute = (on) => {
    muted = !!on;
    for (const m of medias()) {
      if (muted) quiet(m);
      else if (m.__tcMuted) { m.muted = false; m.__tcMuted = false; }
    }
    report();
    return { ok: true };
  };

  // ----- refs: the elements of the last outline an agent can act on
  const refs = new Map();
  let next = 1;
  const refOf = (el) => {
    if (!el.__tcRef) el.__tcRef = 'e' + next++;
    refs.set(el.__tcRef, el);
    return el.__tcRef;
  };
  const byRef = (ref) => {
    const el = refs.get(String(ref || '').replace(/^\[?ref=/, '').replace(/\]$/, ''));
    return el && el.isConnected ? el : null;
  };
  const gone = (ref) => ({ error: `${ref} is not on the page any more: take a new snapshot.` });

  // ----- the outline
  const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD', 'META', 'LINK', 'BR', 'WBR']);
  const TAG_ROLE = {
    A: 'link', BUTTON: 'button', SELECT: 'combobox', TEXTAREA: 'textbox', IMG: 'img', UL: 'list', OL: 'list',
    LI: 'listitem', NAV: 'navigation', MAIN: 'main', HEADER: 'banner', FOOTER: 'contentinfo', ASIDE: 'complementary',
    FORM: 'form', TABLE: 'table', TR: 'row', TD: 'cell', TH: 'columnheader', DIALOG: 'dialog', SUMMARY: 'button',
    ARTICLE: 'article', FIELDSET: 'group', OPTION: 'option', PROGRESS: 'progressbar', HR: 'separator',
    H1: 'heading', H2: 'heading', H3: 'heading', H4: 'heading', H5: 'heading', H6: 'heading', IFRAME: 'iframe',
  };
  const INPUT_ROLE = {
    button: 'button', submit: 'button', reset: 'button', image: 'button', checkbox: 'checkbox', radio: 'radio',
    range: 'slider', number: 'spinbutton', search: 'searchbox', file: 'button', color: 'button',
  };
  const ACTIONABLE = new Set(['link', 'button', 'textbox', 'searchbox', 'checkbox', 'radio', 'combobox', 'slider',
    'spinbutton', 'option', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'tab', 'switch', 'listbox', 'treeitem']);
  // Named by their text, and their content not listed below them.
  const LEAF = new Set(['link', 'button', 'heading', 'tab', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'option',
    'switch', 'treeitem', 'textbox', 'searchbox', 'combobox', 'slider', 'spinbutton', 'checkbox', 'radio', 'img',
    'progressbar', 'separator']);
  const roleOf = (el) => {
    const explicit = (el.getAttribute('role') || '').trim().split(/\s+/)[0];
    if (explicit && explicit !== 'presentation' && explicit !== 'none') return explicit;
    if (el.tagName === 'INPUT') return el.type === 'hidden' ? null : (INPUT_ROLE[el.type] || 'textbox');
    if (el.tagName === 'A' && !el.hasAttribute('href')) return null;
    if (el.tagName === 'IMG' && !(el.getAttribute('alt') || '').trim()) return null;
    if (el.isContentEditable && (!el.parentElement || !el.parentElement.isContentEditable)) return 'textbox';
    return TAG_ROLE[el.tagName] || null;
  };
  const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();
  const clip = (s, n = 100) => (s.length > n ? s.slice(0, n - 1) + '…' : s);
  const textIn = (el) => squash(el.innerText !== undefined ? el.innerText : el.textContent);
  const nameOf = (el, role) => {
    const aria = squash(el.getAttribute('aria-label'));
    if (aria) return aria;
    const by = el.getAttribute('aria-labelledby');
    if (by) {
      const t = squash(by.split(/\s+/).map((id) => { const x = document.getElementById(id); return x ? x.innerText : ''; }).join(' '));
      if (t) return t;
    }
    if (el.labels && el.labels.length) {
      const t = squash([...el.labels].map((l) => l.innerText).join(' '));
      if (t) return t;
    }
    if (el.tagName === 'IMG') return squash(el.getAttribute('alt'));
    if (el.tagName === 'INPUT' && ['button', 'submit', 'reset'].includes(el.type)) return squash(el.value) || el.type;
    if (['textbox', 'searchbox', 'combobox', 'spinbutton', 'slider'].includes(role) && el.tagName !== 'SELECT') {
      return squash(el.getAttribute('placeholder') || el.title);
    }
    if (LEAF.has(role) || role === 'listitem' || role === 'cell' || role === 'columnheader') {
      let t = textIn(el);
      if (!t) t = squash([...el.querySelectorAll('img[alt], svg title, [aria-label]')].map((x) => x.getAttribute('alt') || x.getAttribute('aria-label') || x.textContent).join(' '));
      return t || squash(el.title);
    }
    return squash(el.title);
  };
  const shown = (el) => {
    if (el.getAttribute('aria-hidden') === 'true' || el.hidden) return false;
    const s = getComputedStyle(el);
    return !(s.display === 'none' || s.visibility === 'hidden' || s.visibility === 'collapse');
  };
  const blockish = (el) => {
    const d = getComputedStyle(el).display;
    return !(d === 'inline' || d === 'contents');
  };
  const LIMIT = 30000;

  function outline() {
    refs.clear();
    const lines = [];
    let size = 0;
    let cut = 0;
    let run = '';
    let runDepth = 0;
    const emit = (depth, line) => {
      if (size > LIMIT) { cut++; return; }
      const l = '  '.repeat(depth) + '- ' + line;
      lines.push(l);
      size += l.length + 1;
    };
    const flush = () => {
      const t = squash(run);
      if (t) emit(runDepth, 'text: ' + JSON.stringify(clip(t, 300)));
      run = '';
    };
    const addText = (t, depth) => {
      if (!t.trim()) { if (run) run += ' '; return; }
      if (!run) runDepth = depth;
      run += t;
    };
    const states = (el, role) => {
      const s = [];
      if (role === 'heading') s.push(`[level=${el.tagName.match(/^H(\d)$/) ? el.tagName[1] : (el.getAttribute('aria-level') || 2)}]`);
      if (el.disabled || el.getAttribute('aria-disabled') === 'true') s.push('[disabled]');
      if (el.checked || el.getAttribute('aria-checked') === 'true') s.push('[checked]');
      if (el.getAttribute('aria-selected') === 'true' || (el.tagName === 'OPTION' && el.selected)) s.push('[selected]');
      if (el.getAttribute('aria-pressed') === 'true') s.push('[pressed]');
      const ex = el.getAttribute('aria-expanded');
      if (ex) s.push(`[expanded=${ex}]`);
      if (el.required) s.push('[required]');
      return s.join(' ');
    };
    const valueOf = (el) => {
      if (el.tagName === 'SELECT') return squash([...el.selectedOptions].map((o) => o.text).join(', '));
      if (el.isContentEditable) return clip(textIn(el), 200);
      if (el.type === 'password') return el.value ? '••••' : '';
      return typeof el.value === 'string' ? clip(el.value, 200) : '';
    };
    const walk = (node, depth) => {
      if (node.nodeType === Node.TEXT_NODE) { addText(node.textContent, depth); return; }
      if (node.nodeType !== Node.ELEMENT_NODE) return;
      const el = node;
      if (SKIP.has(el.tagName) || el.id === '__tc_ring' || el.id === '__tc_cursor' || !shown(el)) return;
      if (el.tagName === 'svg' || el.tagName === 'SVG') {
        const label = squash(el.getAttribute('aria-label') || (el.querySelector('title') || {}).textContent);
        if (label) { flush(); emit(depth, `img ${JSON.stringify(clip(label))}`); }
        return;
      }
      const role = roleOf(el);
      const pointer = !role && getComputedStyle(el).cursor === 'pointer' && !(el.parentElement && getComputedStyle(el.parentElement).cursor === 'pointer');
      const block = blockish(el);
      if (!role && !pointer) {
        if (block) flush();
        for (const c of el.childNodes) walk(c, depth);
        if (el.shadowRoot) for (const c of el.shadowRoot.childNodes) walk(c, depth);
        if (block) flush();
        return;
      }
      flush();
      if (role === 'iframe') {
        let doc = null;
        try { doc = el.contentDocument; } catch (e) { /* another site */ }
        emit(depth, `iframe ${JSON.stringify(clip(el.getAttribute('title') || el.src || '', 120))}${doc ? ':' : ''}`);
        if (doc && doc.body) { walk(doc.body, depth + 1); flush(); }
        return;
      }
      const name = pointer ? clip(textIn(el), 100) : clip(nameOf(el, role), 100);
      let line = (pointer ? 'generic' : role) + (name ? ' ' + JSON.stringify(name) : '');
      const st = pointer ? '' : states(el, role);
      if (st) line += ' ' + st;
      if (pointer || ACTIONABLE.has(role) || el.hasAttribute('tabindex') || el.tagName === 'SUMMARY') line += ` [ref=${refOf(el)}]`;
      if (pointer) line += ' [cursor=pointer]';
      if (role === 'link') {
        const href = el.getAttribute('href');
        if (href && !href.startsWith('javascript:')) line += ` [url=${clip(href, 120)}]`;
      }
      if (['textbox', 'searchbox', 'combobox', 'spinbutton', 'slider'].includes(role)) {
        const v = valueOf(el);
        if (v) line += ': ' + JSON.stringify(v);
      }
      if (pointer || LEAF.has(role)) {
        emit(depth, line);
        if (el.tagName === 'SELECT') {
          let n = 0;
          for (const o of el.options) {
            if (n++ >= 25) { emit(depth + 1, `… ${el.options.length - 25} more options`); break; }
            emit(depth + 1, `option ${JSON.stringify(clip(squash(o.text), 80))}${o.selected ? ' [selected]' : ''}`);
          }
        }
        return;
      }
      // A container: its name (when it has one of its own) and what it holds, one level in.
      emit(depth, line + ':');
      for (const c of el.childNodes) walk(c, depth + 1);
      if (el.shadowRoot) for (const c of el.shadowRoot.childNodes) walk(c, depth + 1);
      flush();
    };
    if (document.body) walk(document.body, 0);
    flush();
    let text = lines.join('\n');
    if (cut) text += `\n- … (the page goes on: ${cut} more lines; scroll or act on what is above)`;
    return text || '- (the page shows nothing)';
  }

  // ----- what the user sees the agent do: its cursor going to the element, and a ring around
  // the element with what it does (both made of plain elements: no markup from strings, which
  // a page's Trusted Types would refuse). Both in the app's accent (#2E6BE6), as the ring of
  // the page's frame.
  let cursorTimer = 0;
  const cursor = (x, y) => {
    try {
      let c = document.getElementById('__tc_cursor');
      if (!c) {
        c = document.createElement('div');
        c.id = '__tc_cursor';
        c.style.cssText = 'position:fixed;z-index:2147483647;pointer-events:none;width:22px;height:22px;transition:left .22s ease,top .22s ease,opacity .3s;filter:drop-shadow(0 1px 1.5px rgba(0,0,0,.45))';
        const ns = 'http://www.w3.org/2000/svg';
        const svg = document.createElementNS(ns, 'svg');
        svg.setAttribute('width', '22');
        svg.setAttribute('height', '22');
        svg.setAttribute('viewBox', '0 0 24 24');
        const path = document.createElementNS(ns, 'path');
        path.setAttribute('d', 'M3 2 L3 19 L7.6 14.8 L10.6 21.6 L13.4 20.4 L10.4 13.8 L16.4 13.8 Z');
        path.setAttribute('fill', '#2e6be6');
        path.setAttribute('stroke', '#fff');
        path.setAttribute('stroke-width', '1.3');
        path.setAttribute('stroke-linejoin', 'round');
        svg.appendChild(path);
        c.appendChild(svg);
        c.style.left = innerWidth / 2 + 'px';
        c.style.top = innerHeight / 2 + 'px';
        document.documentElement.appendChild(c);
        c.getBoundingClientRect();
      }
      c.style.opacity = '1';
      c.style.left = x - 3 + 'px';
      c.style.top = y - 2 + 'px';
      clearTimeout(cursorTimer);
      cursorTimer = setTimeout(() => { c.style.opacity = '0'; }, 2500);
    } catch (e) { /* only a courtesy */ }
  };
  let ringTimer = 0;
  const ring = (el, label) => {
    try {
      let box = document.getElementById('__tc_ring');
      if (!box) {
        box = document.createElement('div');
        box.id = '__tc_ring';
        box.style.cssText = 'position:fixed;z-index:2147483647;pointer-events:none;box-sizing:content-box;border:2px solid #2e6be6;border-radius:8px;box-shadow:0 0 0 4px rgba(46,107,230,.25);transition:all .15s ease';
        const tag = document.createElement('span');
        tag.style.cssText = 'position:absolute;left:-2px;top:-22px;height:18px;line-height:18px;background:#2e6be6;color:#fff;font:400 11px ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;padding:0 6px;border-radius:5px;white-space:nowrap';
        box.appendChild(tag);
        document.documentElement.appendChild(box);
      }
      // 4 px out from the element all round.
      const r = el.getBoundingClientRect();
      Object.assign(box.style, { left: r.left - 4 + 'px', top: r.top - 4 + 'px', width: r.width + 4 + 'px', height: r.height + 4 + 'px', display: 'block' });
      box.firstChild.textContent = label;
      clearTimeout(ringTimer);
      ringTimer = setTimeout(() => { box.style.display = 'none'; }, 1400);
    } catch (e) { /* the ring is only a courtesy */ }
  };

  // ----- acting
  const pause = (ms) => new Promise((r) => setTimeout(r, ms));
  // `allowed`: the sites the agent may open (`example.com` or `*.example.com`: that host and
  // its subdomains).
  const listed = (host, allowed) => allowed.some((s) => {
    if (String(s).trim() === '*') return true;
    const h = String(s).trim().replace(/^\*\./, '').toLowerCase();
    return h && (host === h || host.endsWith('.' + h));
  });
  // Where a click on `el` takes the page, when that is a site the agent may not open.
  const leaves = (el, allowed) => {
    const a = el.closest && el.closest('a[href]');
    const form = (el.form || (el.closest && el.closest('form'))) && (el.type === 'submit' || el.tagName === 'BUTTON') ? (el.form || el.closest('form')) : null;
    let url = null;
    if (a && (!a.target || a.target === '_self' || a.target === '_top' || a.target === '_blank')) url = a.href;
    else if (form) url = form.action || location.href;
    if (!url) return null;
    let u;
    try { u = new URL(url, location.href); } catch (e) { return null; }
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
    if (u.host === location.host || listed(u.hostname, allowed)) return null;
    return u.href;
  };
  const mouse = (target, type, x, y) => {
    const init = { bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y, button: 0, buttons: type.endsWith('down') ? 1 : 0, view: window };
    const E = type.startsWith('pointer') ? PointerEvent : MouseEvent;
    target.dispatchEvent(new E(type, type.startsWith('pointer') ? { ...init, pointerType: 'mouse', isPrimary: true, pointerId: 1 } : init));
  };
  async function click(args) {
    const el = byRef(args.ref);
    if (!el) return gone(args.ref);
    const out = !args.force && leaves(el, args.allowed || []);
    if (out) return { leaves: out };
    el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    const r = el.getBoundingClientRect();
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    cursor(x, y);
    ring(el, 'click');
    // The cursor gets there before the click, so the user can follow it.
    await pause(240);
    const top = document.elementFromPoint(x, y);
    const target = top && (top === el || el.contains(top)) ? top : el;
    mouse(target, 'pointerover', x, y); mouse(target, 'mouseover', x, y);
    mouse(target, 'pointerdown', x, y); mouse(target, 'mousedown', x, y);
    if (el.focus) el.focus({ preventScroll: true });
    mouse(target, 'pointerup', x, y); mouse(target, 'mouseup', x, y);
    if (typeof target.click === 'function') target.click(); else mouse(target, 'click', x, y);
    await pause(300);
    const covered = top && target !== top ? ` (another element was over it: ${clip(squash(top.innerText || top.tagName), 60)})` : '';
    return { ok: true, note: covered };
  }
  const setValue = (el, value) => {
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    const desc = Object.getOwnPropertyDescriptor(proto, 'value');
    if (desc && desc.set) desc.set.call(el, value); else el.value = value;
  };
  async function type(args) {
    const el = byRef(args.ref);
    if (!el) return gone(args.ref);
    el.scrollIntoView({ block: 'center', behavior: 'instant' });
    const fr = el.getBoundingClientRect();
    cursor(fr.left + Math.min(24, fr.width / 2), fr.top + fr.height / 2);
    ring(el, 'type');
    await pause(200);
    el.focus();
    const text = String(args.text ?? '');
    if (el.isContentEditable && args.native) {
      // The app puts the text in as typed (the system's input), over the editor's text selected
      // here: an editor (Draft, Lexical, ProseMirror) keeps its own model of the text.
      document.execCommand('selectAll', false);
      await pause(60);
      return { ok: true, insert: true };
    }
    if (el.isContentEditable) {
      document.execCommand('selectAll', false);
      document.execCommand('insertText', false, text);
    } else if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {
      if (el.readOnly || el.disabled) return { error: `${args.ref} can't be typed into (read-only or disabled).` };
      setValue(el, text);
      el.dispatchEvent(new InputEvent('input', { bubbles: true, composed: true, inputType: 'insertText', data: text }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    } else {
      return { error: `${args.ref} is not a text field.` };
    }
    if (args.submit) return press({ key: 'Enter' });
    await pause(150);
    return { ok: true };
  }
  async function select(args) {
    const el = byRef(args.ref);
    if (!el) return gone(args.ref);
    if (el.tagName !== 'SELECT') return { error: `${args.ref} is not a select element.` };
    ring(el, 'select');
    const want = (args.values || []).map(String);
    const picked = [];
    for (const o of el.options) {
      const on = want.includes(o.value) || want.includes(squash(o.text));
      if (on && (el.multiple || picked.length === 0)) { o.selected = true; picked.push(squash(o.text)); } else if (el.multiple) o.selected = false;
    }
    if (!picked.length) return { error: `No option of ${args.ref} is ${JSON.stringify(want)}.` };
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    await pause(150);
    return { ok: true, note: ` Selected: ${picked.join(', ')}.` };
  }
  const CODES = { Enter: 'Enter', Escape: 'Escape', Tab: 'Tab', Backspace: 'Backspace', Delete: 'Delete', ' ': 'Space',
    ArrowUp: 'ArrowUp', ArrowDown: 'ArrowDown', ArrowLeft: 'ArrowLeft', ArrowRight: 'ArrowRight', Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown' };
  const focusable = () => [...document.querySelectorAll('a[href], button, input:not([type=hidden]), select, textarea, [tabindex]:not([tabindex="-1"]), [contenteditable=""], [contenteditable=true]')]
    .filter((e) => !e.disabled && shown(e) && e.getClientRects().length);
  async function press(args) {
    const key = String(args.key || '');
    if (!key) return { error: 'No key given.' };
    const el = document.activeElement && document.activeElement !== document.documentElement ? document.activeElement : document.body;
    const code = CODES[key] || (key.length === 1 ? (/[a-z]/i.test(key) ? 'Key' + key.toUpperCase() : /\d/.test(key) ? 'Digit' + key : '') : key);
    const init = { key, code, bubbles: true, cancelable: true, composed: true };
    const go = el.dispatchEvent(new KeyboardEvent('keydown', init));
    if (go) {
      if (key.length === 1 || key === 'Enter') el.dispatchEvent(new KeyboardEvent('keypress', init));
      const field = (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') && !el.readOnly;
      if (key === 'Enter') {
        if (el.tagName === 'TEXTAREA') {
          setValue(el, el.value + '\n');
          el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertLineBreak' }));
        } else if (el.form && el.tagName === 'INPUT') {
          if (el.form.requestSubmit) el.form.requestSubmit(); else el.form.submit();
        } else if (el.tagName === 'BUTTON' || el.tagName === 'A' || el.getAttribute('role') === 'button') {
          el.click();
        }
      } else if (key === 'Tab') {
        const all = focusable();
        const i = all.indexOf(el);
        const to = all[(i + 1) % Math.max(all.length, 1)];
        if (to) to.focus();
      } else if (key === ' ' && (el.tagName === 'BUTTON' || el.type === 'checkbox' || el.type === 'radio')) {
        el.click();
      } else if (field && key.length === 1) {
        const s = el.selectionStart ?? el.value.length;
        const e = el.selectionEnd ?? el.value.length;
        setValue(el, el.value.slice(0, s) + key + el.value.slice(e));
        try { el.setSelectionRange(s + 1, s + 1); } catch (x) { /* not every input has a caret */ }
        el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: key }));
      } else if (field && key === 'Backspace') {
        const s = el.selectionStart ?? el.value.length;
        const e = el.selectionEnd ?? el.value.length;
        const from = s === e ? Math.max(0, s - 1) : s;
        setValue(el, el.value.slice(0, from) + el.value.slice(e));
        el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'deleteContentBackward' }));
      }
    }
    el.dispatchEvent(new KeyboardEvent('keyup', init));
    await pause(250);
    return { ok: true };
  }
  async function wait(args) {
    const limit = Math.min(Number(args.time) > 0 ? Number(args.time) : 30, 60) * 1000;
    const end = Date.now() + limit;
    const has = (t) => document.body && document.body.innerText.includes(t);
    if (!args.text && !args.textGone) { await pause(limit); return { ok: true }; }
    for (;;) {
      if ((!args.text || has(args.text)) && (!args.textGone || !has(args.textGone))) return { ok: true };
      if (Date.now() > end) {
        return { error: args.text ? `“${args.text}” did not show within ${limit / 1000} s.` : `“${args.textGone}” was still there after ${limit / 1000} s.` };
      }
      await pause(200);
    }
  }

  // The box that scrolls `el` (the nearest ancestor that can), else the page.
  const scroller = (el) => {
    for (let e = el; e && e !== document.body && e !== document.documentElement; e = e.parentElement) {
      const s = getComputedStyle(e);
      if (/(auto|scroll)/.test(s.overflowY + s.overflowX) && (e.scrollHeight > e.clientHeight || e.scrollWidth > e.clientWidth)) return e;
    }
    return null;
  };
  async function scroll(args) {
    const el = args.ref ? byRef(args.ref) : null;
    if (args.ref && !el) return gone(args.ref);
    if (el && !args.direction) {
      el.scrollIntoView({ block: 'center', behavior: 'instant' });
      ring(el, 'scroll');
      await pause(250);
      return { ok: true };
    }
    const box = el ? scroller(el) : null;
    const n = Number(args.pages) > 0 ? Number(args.pages) : 1;
    const h = (box ? box.clientHeight : innerHeight) * 0.85 * n;
    const w = (box ? box.clientWidth : innerWidth) * 0.85 * n;
    const d = String(args.direction || 'down');
    const by = { left: d === 'left' ? -w : d === 'right' ? w : 0, top: d === 'up' ? -h : d === 'down' ? h : 0, behavior: 'instant' };
    if (box) box.scrollBy(by); else window.scrollBy(by);
    await pause(300);
    const root = document.scrollingElement || document.documentElement;
    const at = box ? box.scrollTop : scrollY;
    const end = box ? box.scrollHeight - box.clientHeight : root.scrollHeight - innerHeight;
    return { ok: true, note: ` Now at ${Math.round(at)} of ${Math.max(0, Math.round(end))} px.` };
  }

  // Where a real click (a mouse event the app sends) lands on `el`: its middle, in the page's
  // viewport (an element in a frame of this site counts its frame's place), after it is brought
  // into view and shown to the user (the cursor, a ring with `label`). For a click, a link or
  // form to a site the agent may not open answers `leaves` instead.
  async function aim(args) {
    const el = byRef(args.ref);
    if (!el) return gone(args.ref);
    if (args.label === 'click') {
      const out = !args.force && leaves(el, args.allowed || []);
      if (out) return { leaves: out };
    }
    el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    const r = el.getBoundingClientRect();
    let x = r.left + r.width / 2;
    let y = r.top + r.height / 2;
    for (let w = el.ownerDocument.defaultView; w && w !== window && w.frameElement; w = w.parent) {
      const f = w.frameElement.getBoundingClientRect();
      x += f.left; y += f.top;
    }
    if (el.ownerDocument === document) cursor(x, y);
    ring(el, args.label || 'click');
    await pause(220);
    const top = document.elementFromPoint(x, y);
    const covered = top && el.ownerDocument === document && top !== el && !el.contains(top) && !top.contains(el)
      ? ` (another element was over it: ${clip(squash(top.innerText || top.tagName), 60)})` : '';
    return { x, y, note: covered, draggable: !!(el.closest && el.closest('[draggable="true"]')) };
  }
  // A drag with the page's own drag and drop (an element that is draggable), from one element
  // to another: the events a person's drag makes, with one shared DataTransfer.
  async function dragHtml5(args) {
    const a = byRef(args.ref);
    const b = byRef(args.to);
    if (!a) return gone(args.ref);
    if (!b) return gone(args.to);
    const src = a.closest('[draggable="true"]') || a;
    const dt = new DataTransfer();
    const at = (el) => { const r = el.getBoundingClientRect(); return { clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 }; };
    const fire = (el, type) => el.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, composed: true, dataTransfer: dt, ...at(el) }));
    ring(src, 'drag');
    fire(src, 'dragstart');
    await pause(60);
    fire(b, 'dragenter');
    fire(b, 'dragover');
    ring(b, 'drop');
    await pause(60);
    fire(b, 'drop');
    fire(src, 'dragend');
    await pause(200);
    return { ok: true };
  }
  // The agent's cursor at a point of the page (a click there comes next).
  async function point(args) {
    cursor(Number(args.x) || 0, Number(args.y) || 0);
    await pause(220);
    const top = document.elementFromPoint(Number(args.x) || 0, Number(args.y) || 0);
    return { ok: true, note: top ? ` (on ${clip(squash(top.innerText || top.getAttribute('aria-label') || top.tagName), 60)})` : '' };
  }

  // ----- where the caret was in the editor that has the keyboard, while the page had it.
  // WebKit clears the selection when the page loses the keyboard (the agent's call ends, the
  // user clicks in the app) and puts the caret at the start of the editor when it gets it back:
  // the agent's next keys go where its caret was (`caret`), not before the text. A text field
  // (input, textarea) keeps its own selection.
  let caret = null;
  // The selection was cleared (the page lost the keyboard): the next caret at the start of the
  // editor is WebKit's, as the page gets the keyboard back.
  let cleared = false;
  // Offsets count the characters of the editor's text nodes (block breaks are not characters).
  const offsetIn = (root, node, off) => {
    const r = document.createRange();
    r.selectNodeContents(root);
    try { r.setEnd(node, off); } catch (e) { return null; }
    return r.toString().length;
  };
  const pointAt = (root, n) => {
    const w = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let t, last = null;
    while ((t = w.nextNode())) {
      if (n <= t.data.length) return [t, n];
      n -= t.data.length;
      last = t;
    }
    return last ? [last, last.data.length] : [root, root.childNodes.length];
  };
  document.addEventListener('selectionchange', () => {
    const el = document.activeElement;
    if (!el || !el.isContentEditable) return;
    const s = getSelection();
    if (!s.rangeCount) { cleared = true; return; }
    if (!el.contains(s.anchorNode)) return;
    const a = offsetIn(el, s.anchorNode, s.anchorOffset);
    const f = offsetIn(el, s.focusNode, s.focusOffset);
    if (a === null || f === null) return;
    const restored = cleared && a === 0 && f === 0;
    cleared = false;
    if (restored && caret && caret.el === el) return;
    caret = { el, a, f };
  });
  async function restoreCaret() {
    const el = document.activeElement;
    if (!caret || el !== caret.el || !el.isConnected) return { ok: true };
    const s = getSelection();
    const at = s.rangeCount && el.contains(s.anchorNode) ? [offsetIn(el, s.anchorNode, s.anchorOffset), offsetIn(el, s.focusNode, s.focusOffset)] : [];
    if (at[0] === caret.a && at[1] === caret.f) return { ok: true };
    const [an, ao] = pointAt(el, caret.a);
    const [fn, fo] = pointAt(el, caret.f);
    s.setBaseAndExtent(an, ao, fn, fo);
    // The editor hears of it (selectionchange) before the next key.
    await pause(30);
    return { ok: true };
  }

  async function run(name, args) {
    try {
      switch (name) {
        case 'caret': return await restoreCaret();
        case 'snapshot': return { text: outline(), title: document.title, url: location.href };
        case 'aim': return await aim(args);
        case 'drag5': return await dragHtml5(args);
        case 'point': return await point(args);
        case 'click': return await click(args);
        case 'type': return await type(args);
        case 'select': return await select(args);
        case 'press': return await press(args);
        case 'wait': return await wait(args);
        case 'scroll': return await scroll(args);
        case 'console': return { text: log.length ? log.join('\n') : 'No console messages.' };
        case 'mute': return mute(args.on);
        default: return { error: `Unknown action ${name}.` };
      }
    } catch (e) {
      return { error: `${name} failed in the page: ${e && e.message ? e.message : e}` };
    }
  }
  return { run, ring };
})();
