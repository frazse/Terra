// Terra's own UI: a cog next to Biketerra's theme toggle that opens the addons modal.
// Built in (not an addon) so you can always get back to it, even with every addon off.
(() => {
  if (window.top !== window || window.__terraUi) return;
  const core = window.__terraCore, addons = window.__terraAddons;
  if (!core || !addons) return;
  window.__terraUi = true;

  const CSS = `
.terra-overlay { position: fixed; inset: 0; z-index: 2147483647; display: flex; align-items: center; justify-content: center;
  background: rgba(0,0,0,.55); backdrop-filter: blur(2px); font: 14px system-ui, sans-serif; }
.terra-modal { width: min(560px, calc(100vw - 32px)); max-height: calc(100vh - 64px); display: flex; flex-direction: column;
  background: #15151a; color: #eee; border-radius: 14px; box-shadow: 0 20px 60px rgba(0,0,0,.6); overflow: hidden; }
.terra-head { display: flex; align-items: center; justify-content: space-between; padding: 16px 20px; border-bottom: 1px solid #25252d; }
.terra-head h3 { margin: 0; font-size: 17px; color: #fff; }
.terra-x { background: none; border: 0; color: #999; font-size: 24px; line-height: 1; cursor: pointer; padding: 0 4px; }
.terra-x:hover { color: #fff; }
.terra-body { padding: 4px 20px; overflow: auto; }
.terra-item { border-bottom: 1px solid #25252d; }
.terra-item:last-child { border-bottom: 0; }
.terra-row { display: flex; gap: 14px; align-items: center; padding: 14px 0; }
.terra-link { background: none; border: 0; padding: 0; margin-top: 8px; color: #8fa3b8; font: inherit; font-size: 12px; cursor: pointer; text-decoration: underline; }
.terra-link:hover { color: #fff; }
.terra-settings { padding: 0 0 16px; }
.terra-field { margin-top: 12px; }
.terra-field label { display: block; margin-bottom: 4px; font-size: 12px; color: #bbb; }
.terra-input { width: 100%; box-sizing: border-box; padding: 7px 9px; border-radius: 6px; font: inherit;
  background-color: #2a2a33 !important; color: #eee !important; border: 1px solid #3a3a44 !important; }
.terra-field input[type=checkbox] { width: auto; }
.terra-help { margin-top: 3px; font-size: 11px; color: #777; }
.terra-status { margin-left: 8px; font-size: 11px; color: #7ddc8a; }
.terra-info { flex: 1; min-width: 0; }
.terra-name { font-weight: 600; color: #fff; }
.terra-ver { color: #888; font-weight: 400; font-size: 12px; margin-left: 6px; }
.terra-dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: #555; margin-right: 8px; }
.terra-dot.on { background: #4ade80; }
.terra-desc { color: #b0b0bb; font-size: 12px; margin-top: 3px; }
.terra-meta { color: #6f6f7c; font-size: 11px; margin-top: 4px; word-break: break-all; }
.terra-empty { color: #aaa; padding: 16px 0; }
.terra-empty code { display: block; margin-top: 6px; color: #ffd9d2; word-break: break-all; }
.terra-switch { position: relative; width: 44px; height: 24px; flex: none; }
.terra-switch input { position: absolute; inset: 0; width: 100%; height: 100%; margin: 0; opacity: 0; cursor: pointer; z-index: 1; }
.terra-switch span { position: absolute; inset: 0; border-radius: 999px; background: #444; transition: background .15s; }
.terra-switch span::after { content: ''; position: absolute; top: 3px; left: 3px; width: 18px; height: 18px; border-radius: 50%;
  background: #fff; transition: transform .15s; }
.terra-switch input:checked + span { background: #ff6b57; }
.terra-switch input:checked + span::after { transform: translateX(20px); }
.terra-switch input:focus-visible + span { outline: 2px solid #fff; outline-offset: 2px; }
.terra-foot { display: flex; flex-direction: column; gap: 8px; padding: 12px 20px 16px; border-top: 1px solid #25252d; color: #888; font-size: 12px; }
.terra-foot-dirs { min-width: 0; word-break: break-all; }
.terra-foot-bottom { display: flex; align-items: center; justify-content: space-between; width: 100%; }
.terra-foot-ver { color: #aaa; font-weight: 600; font-size: 12px; }
.terra-btn { flex: none; background: #2a2a33; color: #eee; border: 0; border-radius: 6px; padding: 7px 14px; font: inherit; cursor: pointer; }
.terra-btn:hover { background: #363642; }
`;
  let sheeted = false;
  const ensureCss = () => {
    if (sheeted) return;
    sheeted = true;
    try {
      const s = new CSSStyleSheet();
      s.replaceSync(CSS);
      document.adoptedStyleSheets = [...document.adoptedStyleSheets, s];
    } catch {
      const el = document.createElement('style');
      el.textContent = CSS;
      document.head.append(el);
    }
  };

  const h = (tag, props = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(props)) {
      if (k === 'class') e.className = v;
      else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v);
    }
    e.append(...kids);
    return e;
  };

  /* ---------- modal ---------- */
  let overlay = null, body = null, foot = null;
  const dots = new Map(); // addon id -> dot element
  const settingsToggles = new Map(); // addon id -> open/close its settings section
  const paintDots = () => { for (const [id, dot] of dots) dot.classList.toggle('on', addons.active.has(id)); };

  // Settings form for one addon. Password fields are write-only: a saved secret is shown as a placeholder only.
  async function fillSettings(box, a) {
    let res;
    try { res = await core.call('addon_settings_get', { id: a.id }); }
    catch (e) { box.textContent = 'Could not load settings: ' + e; return; }
    box.replaceChildren();
    const SAVED = 'saved \u2022\u2022\u2022\u2022\u2022\u2022  (type to replace)';
    for (const def of a.settings) {
      const secret = def.type === 'password';
      const stored = res.values[def.key];
      const isSet = secret ? res.secretsSet.includes(def.key) : stored !== undefined && stored !== null;
      const input = h('input', {
        class: 'terra-input', autocomplete: 'off', spellcheck: 'false',
        type: def.type === 'number' ? 'number' : secret ? 'password' : def.type === 'checkbox' ? 'checkbox' : 'text',
      });
      if (def.type === 'checkbox') input.checked = !!stored;
      else if (secret) input.placeholder = isSet ? SAVED : 'not set';
      else input.value = stored === undefined || stored === null ? '' : String(stored);

      const status = h('span', { class: 'terra-status' });
      const clearBtn = h('button', { class: 'terra-link', type: 'button' }, 'clear saved value');
      clearBtn.style.display = secret && isSet ? '' : 'none';
      const save = async (value) => {
        try {
          await core.call('addon_settings_set', { id: a.id, key: def.key, value });
          status.textContent = value === '' ? 'cleared' : 'saved';
          if (secret) {
            input.value = '';
            input.placeholder = value === '' ? 'not set' : SAVED;
            clearBtn.style.display = value === '' ? 'none' : '';
          }
          core.bus.emit('settings:' + a.id, { key: def.key }); // running addons can react
        } catch (e) { status.textContent = 'error: ' + e; }
        setTimeout(() => { status.textContent = ''; }, 2500);
      };
      clearBtn.addEventListener('click', () => save(''));
      input.addEventListener('change', () => {
        if (def.type === 'checkbox') return save(input.checked);
        if (secret && input.value === '') return; // an empty field keeps what is saved
        return save(def.type === 'number' ? (input.value === '' ? '' : Number(input.value)) : input.value.trim());
      });
      box.append(h('div', { class: 'terra-field' },
        h('label', {}, def.label || def.key, status),
        input,
        secret ? clearBtn : '',
        def.description ? h('div', { class: 'terra-help' }, def.description) : ''));
    }
  }

  async function render() {
    let res;
    try { res = await core.call('addons_list'); }
    catch (e) { body.textContent = 'Could not load addons: ' + e; return; }
    dots.clear();
    settingsToggles.clear();
    body.replaceChildren();
    if (!res.addons.length) {
      body.append(h('div', { class: 'terra-empty' }, 'No addons found. Put addon folders in:', ...res.dirs.map((d) => h('code', {}, d))));
    }
    for (const a of res.addons) {
      const dot = h('span', { class: 'terra-dot' });
      dots.set(a.id, dot);
      const input = h('input', { type: 'checkbox', 'aria-label': 'Enable ' + a.name });
      input.checked = a.enabled;
      input.addEventListener('change', async () => {
        try {
          await core.call('addons_set_enabled', { id: a.id, enabled: input.checked });
          addons.refresh(); // starts or stops it right away
        } catch (e) {
          input.checked = !input.checked;
          core.log('terra-ui', 'toggle failed: ' + e);
        }
        setTimeout(paintDots, 600);
        setTimeout(paintDots, 1800);
      });

      const box = h('div', { class: 'terra-settings', hidden: '' });
      let toggleBtn = '';
      if (a.settings && a.settings.length) {
        toggleBtn = h('button', { class: 'terra-link', type: 'button' }, 'Settings \u25be');
        const toggle = async (force) => {
          const open = force === undefined ? box.hidden : force;
          if (open) { await fillSettings(box, a); box.hidden = false; toggleBtn.textContent = 'Settings \u25b4'; }
          else { box.hidden = true; toggleBtn.textContent = 'Settings \u25be'; }
        };
        toggleBtn.addEventListener('click', () => toggle());
        settingsToggles.set(a.id, toggle);
      }

      body.append(h('div', { class: 'terra-item' },
        h('div', { class: 'terra-row' },
          h('div', { class: 'terra-info' },
            h('div', { class: 'terra-name' }, dot, a.name, h('span', { class: 'terra-ver' }, 'v' + a.version)),
            a.description ? h('div', { class: 'terra-desc' }, a.description) : '',
            h('div', { class: 'terra-meta' }, a.id + ' \u00b7 ' + a.matches.join(', ') + (a.defaultEnabled ? '' : ' \u00b7 off by default')),
            toggleBtn),
          h('label', { class: 'terra-switch' }, input, h('span'))),
        box));
    }
    const dirsEl = foot.querySelector('.terra-foot-dirs');
    if (dirsEl) dirsEl.textContent = res.dirs.length ? 'Addon folders: ' + res.dirs.join('  \u00b7  ') : '';
    const verEl = foot.querySelector('.terra-foot-ver');
    if (verEl) verEl.textContent = 'Terra ' + (res.version || '0.3.1');
    paintDots();
  }

  const onKey = (e) => { if (e.key === 'Escape') { e.stopPropagation(); closeModal(); } };
  function closeModal() {
    if (!overlay) return;
    overlay.remove();
    overlay = null;
    removeEventListener('keydown', onKey, true);
  }
  async function openModal(opts) {
    if (overlay) { if (opts && opts.expand) settingsToggles.get(opts.expand)?.(true); return; }
    ensureCss();
    body = h('div', { class: 'terra-body' }, 'Loading\u2026');
    foot = h('div', { class: 'terra-foot' },
      h('div', { class: 'terra-foot-dirs' }),
      h('div', { class: 'terra-foot-bottom' },
        h('div', { class: 'terra-foot-ver' }, 'Terra 0.3.1'),
        h('button', { class: 'terra-btn', type: 'button', onclick: () => location.reload() }, 'Reload page')
      )
    );
    overlay = h('div', { class: 'terra-overlay' },
      h('div', { class: 'terra-modal', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Terra addons' },
        h('div', { class: 'terra-head' }, h('h3', {}, 'Terra addons'),
          h('button', { class: 'terra-x', type: 'button', 'aria-label': 'Close', onclick: closeModal }, '\u00d7')),
        body, foot));
    overlay.addEventListener('click', (e) => { if (e.target === overlay) closeModal(); });
    addEventListener('keydown', onKey, true);
    document.body.append(overlay);
    await render();
    if (opts && opts.expand) settingsToggles.get(opts.expand)?.(true); // e.g. an addon asking for its settings
  }

  /* ---------- cog next to the theme toggle ---------- */
  const GEAR = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">' +
    '<circle cx="12" cy="12" r="3"></circle>' +
    '<path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path></svg>';
  let cog = null;
  function placeCog() {
    const theme = document.querySelector('.nav-profile .theme-toggle') || document.querySelector('.theme-toggle');
    if (!theme) return; // no header on this page (e.g. rides)
    if (!cog) {
      cog = h('button', { type: 'button', 'aria-label': 'Terra addons', title: 'Terra addons' });
      cog.innerHTML = GEAR;
      cog.addEventListener('click', () => openModal());
    }
    const cls = theme.className + ' terra-cog'; // same classes as the theme toggle = same look and hover
    if (cog.className !== cls) cog.className = cls;
    if (theme.nextElementSibling !== cog) theme.after(cog);
  }
  core.bus.on('terra:open-addons', (e) => openModal(e)); // addons can open their settings (terra.settings.open)
  setInterval(placeCog, 500); // the page re-renders its header now and then
  placeCog();
})();
