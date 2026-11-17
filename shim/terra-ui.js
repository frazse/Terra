// Terra's own UI: a cog in Biketerra's header (next to "Help"; old builds: next to the theme toggle) that opens the addons modal.
// Built in (not an addon) so you can always get back to it, even with every addon off.
(() => {
  if (window.top !== window || window.__terraUi) return;
  const core = window.__terraCore, addons = window.__terraAddons;
  if (!core || !addons) return;
  window.__terraUi = true;

  const CSS = `
.terra-overlay { position: fixed; inset: 0; z-index: 2147483647; display: flex; align-items: center; justify-content: center;
  background: rgba(0,0,0,.65); font: 14px system-ui, sans-serif; }
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
.terra-field input[type=color] { width: 110px; height: 32px; padding: 2px; box-sizing: border-box; cursor: pointer; }
.terra-help { margin-top: 3px; font-size: 11px; color: #777; }
.terra-preview { margin-bottom: 14px; padding: 12px; min-height: 44px; box-sizing: border-box;
  background: #1d1d24; border: 1px dashed #3d3d49; border-radius: 8px; overflow: hidden; }
.terra-preview:empty::after { content: 'no preview (is the addon running?)'; font-size: 11px; color: #666; }
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
.terra-cog { display: inline-flex; align-items: center; padding: 0 6px; margin: 0 2px; cursor: pointer; }
/* .nav-profile/.header prefix (specificity 0,2,0) so the page's own button rules can't re-apply a background */
.nav-profile .terra-cog, header .terra-cog { background: transparent; border: 0; color: inherit; }
.terra-cog:hover { opacity: .75; }
.terra-datadir { padding: 12px 20px; border-top: 1px solid #25252d; font-size: 12px; color: #888; }
.terra-datadir .terra-dd-path { font-family: monospace; font-size: 11px; color: #aaa; word-break: break-all; margin: 4px 0; }
.terra-datadir .terra-dd-hint { color: #e6a23c; margin-bottom: 6px; }
.terra-datadir .terra-dd-actions { display: flex; gap: 8px; margin-top: 8px; flex-wrap: wrap; }
.terra-dd-restart { color: #7ddc8a; margin-top: 6px; font-size: 12px; }
.terra-dd-error { color: #f87171; margin-top: 6px; font-size: 12px; }
.terra-btn-primary { flex: none; background: #ff6b57; color: #fff; border: 0; border-radius: 6px; padding: 7px 14px; font: inherit; font-weight: 600; cursor: pointer; }
.terra-btn-primary:hover { background: #ff5740; }
.terra-btn[disabled], .terra-btn-primary[disabled] { opacity: .6; cursor: default; }
.terra-setup-body { padding: 16px 20px; color: #ccc; line-height: 1.5; }
.terra-setup-body p { margin: 0 0 10px; }
.terra-setup-body .terra-dd-path { font-family: monospace; font-size: 11px; color: #aaa; word-break: break-all; margin: 8px 0; }
.terra-setup-body .terra-dd-hint { color: #e6a23c; font-size: 12px; margin: 8px 0; }
.terra-setup-foot { display: flex; gap: 10px; justify-content: flex-end; flex-wrap: wrap; padding: 12px 20px 16px; border-top: 1px solid #25252d; }
.terra-btn-danger { background: #3a1a1a; border: 1px solid #6b2020; color: #e06060; border-radius: 6px; padding: 4px 10px; cursor: pointer; font-size: 12px; }
.terra-btn-danger:hover { background: #4a2020; }
canvas, svg, .leaflet-container, [class*="map"], [class*="elevation"], [class*="profile"] {
  will-change: transform;
  transform: translateZ(0);
  backface-visibility: hidden;
}
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
  const previews = new Map(); // addon id -> live preview slot element (only while its settings are open)
  const paintDots = () => { for (const [id, dot] of dots) dot.classList.toggle('on', addons.active.has(id)); };

  // Ask the addon to (re)paint its live preview into `slot`. `slot === null` hides it.
  // Emitted after 'settings:<id>' so addons see fresh values first.
  const firePreview = (id, slot) => core.bus.emit('settings:preview:' + id, { slot: slot || null });

  const colorOf = (v) => typeof v === 'string' && /^#[0-9a-f]{6}$/i.test(v.trim()) ? v.trim() : '#000000';

  // Settings form for one addon. Password fields are write-only: a saved secret is shown as a placeholder only.
  async function fillSettings(box, a) {
    let res;
    try { res = await core.call('addon_settings_get', { id: a.id }); }
    catch (e) { box.textContent = 'Could not load settings: ' + e; return; }
    box.replaceChildren();
    const SAVED = 'saved \u2022\u2022\u2022\u2022\u2022\u2022  (type to replace)';
    const cur = def => {
      const v = res.values[def.key];
      return v === undefined || v === null ? (def.default !== undefined && def.default !== null ? def.default : '') : v;
    };
    // Always render the slot: when the addon is active its paint fills it, and
    // when it isn't (opened off a non-matching page) the .terra-preview:empty
    // placeholder tells the user that. Logging the gate helps debugging.
    if (!addons.active.has(a.id)) core.log('terra-ui', `preview slot created for ${a.id} but addon not active — will show placeholder`);
    const slot = h('div', { class: 'terra-preview' });
    box.append(slot); previews.set(a.id, slot);
    for (const def of a.settings) {
      const secret = def.type === 'password';
      const stored = res.values[def.key];
      const isSet = secret ? res.secretsSet.includes(def.key) : stored !== undefined && stored !== null;
      let input;
      if (def.type === 'select') {
        input = h('select', { class: 'terra-input' });
        const opts = def.options || [];
        for (const o of opts) input.append(h('option', { value: o }, String(o)));
        const v = cur(def);
        if (v !== '' && !opts.includes(v)) input.append(h('option', { value: String(v) }, String(v)));
        if (v !== '') input.value = String(v);
      } else if (def.type === 'color') {
        input = h('input', { type: 'color' });
        input.value = colorOf(cur(def));
      } else {
        input = h('input', {
          class: 'terra-input', autocomplete: 'off', spellcheck: 'false',
          type: def.type === 'number' ? 'number' : secret ? 'password' : def.type === 'checkbox' ? 'checkbox' : 'text',
        });
        if (def.type === 'checkbox') input.checked = !!stored;
        else if (secret) input.placeholder = isSet ? SAVED : 'not set';
        else input.value = stored === undefined || stored === null ? '' : String(stored);
      }

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
          const p = previews.get(a.id);
          if (p) firePreview(a.id, p); // keep the live preview in sync
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
    if (slot) { core.log('terra-ui', `firing preview for ${a.id}`); firePreview(a.id, slot); } // the addon paints its live sample
  }

  function hidePreviews() {
    for (const [id, p] of previews) { previews.delete(id); firePreview(id, null); }
  }

  async function render() {
    let res;
    try { res = await core.call('addons_list'); }
    catch (e) { body.textContent = 'Could not load addons: ' + e; return; }
    dots.clear();
    settingsToggles.clear();
    hidePreviews();
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
          else if (!box.hidden) { box.hidden = true; toggleBtn.textContent = 'Settings \u25be';
            const p = previews.get(a.id);
            if (p) { previews.delete(a.id); firePreview(a.id, null); }
          }
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
    if (verEl) verEl.textContent = 'Terra' + (res.version ? ' ' + res.version : '');
    paintDots();
  }

  const onKey = (e) => { if (e.key === 'Escape') { e.stopPropagation(); closeModal(); } };
  function closeModal() {
    if (!overlay) return;
    overlay.remove();
    overlay = null;
    hidePreviews();
    removeEventListener('keydown', onKey, true);
  }
  async function openModal(opts) {
    if (overlay) { if (opts && opts.expand) settingsToggles.get(opts.expand)?.(true); return; }
    ensureCss();
    body = h('div', { class: 'terra-body' }, 'Loading\u2026');

    // Data folder section (Android: picker + reset; other platforms: info + reset)
    const ddSection = h('div', { class: 'terra-datadir' },
      h('strong', {}, 'Data folder'));
    foot = h('div', { class: 'terra-foot' },
      ddSection,
      h('div', { class: 'terra-foot-dirs' }),
      h('div', { class: 'terra-foot-bottom' },
        h('div', { class: 'terra-foot-ver' }, 'Terra 0.9.0'),
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

    // Populate the data folder section asynchronously
    (async () => {
      try {
        const st = await core.call('terra_data_dir_status', {});
        const pathEl = h('div', { class: 'terra-dd-path' }, st.path || 'unknown');
        const statusMsg = h('div', {});
        ddSection.append(pathEl);
        if (st.isDefault && navigator.userAgent.includes('Android')) {
          ddSection.append(h('div', { class: 'terra-dd-hint' },
            'This is a hidden Android folder. Choose a visible folder so your file manager can see it.'));
        }
        ddSection.append(statusMsg);
        const actions = h('div', { class: 'terra-dd-actions' });
        if (navigator.userAgent.includes('Android')) {
          const pickBtn = h('button', { class: 'terra-btn', type: 'button' }, 'Choose folder\u2026');
          pickBtn.addEventListener('click', async () => {
            pickBtn.disabled = true;
            pickBtn.textContent = 'Waiting for picker\u2026';
            statusMsg.className = '';
            statusMsg.textContent = '';
            try {
              const res = await core.call('terra_pick_data_dir', {});
              if (res.status === 'success') {
                pathEl.textContent = res.path;
                ddSection.querySelector('.terra-dd-hint')?.remove();
                statusMsg.className = 'terra-dd-restart';
                statusMsg.textContent = 'Folder saved. Please close and reopen Terra for the new folder to take effect.';
                core.log('terra-ui', 'data dir changed to: ' + res.path);
              } else if (res.status !== 'cancelled') {
                statusMsg.className = 'terra-dd-error';
                statusMsg.textContent = res.message || ('Picker ' + res.status);
                core.log('terra-ui', 'data dir picker: ' + (res.message || res.status));
              }
            } catch (e) {
              statusMsg.className = 'terra-dd-error';
              statusMsg.textContent = 'Error: ' + e;
              core.log('terra-ui', 'data dir picker error: ' + e);
            }
            pickBtn.disabled = false;
            pickBtn.textContent = 'Choose folder\u2026';
          });
          actions.append(pickBtn);
        }
        const resetBtn = h('button', { class: 'terra-btn-danger', type: 'button' }, 'Reset all data');
        resetBtn.addEventListener('click', async () => {
          if (!confirm('This will DELETE all Terra addons, addon settings, and remembered Bluetooth devices.\n\nThis cannot be undone. Continue?')) return;
          if (!confirm('Are you absolutely sure? Everything will be wiped: addons/, addon-data/, devices.json.')) return;
          try {
            const res = await core.call('terra_reset_all_data', {});
            core.log('terra-ui', 'reset all data: ' + JSON.stringify(res));
            alert('Terra data reset. Remove ' + (res.removed || []).join(', ') + ' from ' + res.path + '.\n\nReload the page to apply.');
          } catch (e) { alert('Reset failed: ' + e); }
        });
        actions.append(resetBtn);
        ddSection.append(actions);
      } catch (e) {
        // Command not available (e.g. older build) — show a minimal hint
        ddSection.append(h('div', { class: 'terra-dd-path' }, 'data folder feature unavailable'));
      }
    })();

    await render();
    if (opts && opts.expand) settingsToggles.get(opts.expand)?.(true); // e.g. an addon asking for its settings
  }

  /* ---------- cog next to the header's "Help" link (old builds: next to the theme toggle) ---------- */
  const GEAR = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">' +
    '<circle cx="12" cy="12" r="3"></circle>' +
    '<path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path></svg>';
  let cog = null;
  function placeCog() {
    // Redesigned header: the theme toggle is gone, so anchor next to the "Help" link in .nav-profile.
    const help = document.querySelector('.nav-profile a[href="/help-center"]') ||
      Array.from(document.querySelectorAll('.nav-profile a')).find((a) => a.textContent.trim().toLowerCase() === 'help');
    const anchor = help ||
      document.querySelector('.nav-profile .theme-toggle') || // older builds
      document.querySelector('.theme-toggle');
    if (!anchor) return; // no header on this page (e.g. rides)
    if (!cog) {
      cog = h('button', { type: 'button', 'aria-label': 'Terra addons', title: 'Terra addons' });
      cog.innerHTML = GEAR;
      cog.addEventListener('click', () => openModal());
    }
    // Style the cog explicitly (see .terra-cog in the CSS block): do NOT copy the anchor's
    // classes — the Svelte scope hash on them also matches the component's own button rules
    // (the account trigger's border/background), which gave the gear a stray box.
    if (cog.className !== 'terra-cog') cog.className = 'terra-cog';
    if (anchor.nextElementSibling !== cog) anchor.after(cog);
  }
  let setupChecked = false;
  async function maybePromptDataDir() {
    if (setupChecked || !navigator.userAgent.includes('Android') || !document.body) return;
    setupChecked = true;
    try {
      const st = await core.call('terra_data_dir_status', {});
      if (!st || !st.isDefault || st.prompted || overlay) return;
      ensureCss();

      const msgEl = h('div', {});
      const permNote = st.hasStoragePermission === false
        ? h('p', { class: 'terra-dd-hint' },
            'Android will first ask you to allow "All files access" for Terra so it can read and write your chosen folder.')
        : null;

      const bodyEl = h('div', { class: 'terra-setup-body' },
        h('p', {}, 'Choose a visible folder on your device where Terra will store its addons, addon data, and remembered Bluetooth devices (devices.json).'),
        h('div', { class: 'terra-dd-path' }, 'Current default: ' + (st.path || 'Android/data')),
        ...(permNote ? [permNote] : []),
        msgEl
      );

      let setupOverlay = null;
      const closeSetup = () => { if (setupOverlay) { setupOverlay.remove(); setupOverlay = null; } };

      const pickBtn = h('button', { class: 'terra-btn-primary', type: 'button' }, 'Choose folder\u2026');
      const skipBtn = h('button', { class: 'terra-btn', type: 'button' }, 'Use default folder');
      const footEl = h('div', { class: 'terra-setup-foot' }, skipBtn, pickBtn);

      pickBtn.addEventListener('click', async () => {
        pickBtn.disabled = true;
        skipBtn.disabled = true;
        pickBtn.textContent = 'Waiting for picker\u2026';
        msgEl.className = '';
        msgEl.textContent = '';
        try {
          const res = await core.call('terra_pick_data_dir', {});
          if (res.status === 'success') {
            msgEl.className = 'terra-dd-restart';
            msgEl.textContent = 'Folder set to ' + res.path + '. Please close and reopen Terra for the new folder to take effect.';
            footEl.replaceChildren(
              h('button', { class: 'terra-btn-primary', type: 'button', onclick: closeSetup }, 'OK')
            );
            return;
          }
          if (res.status !== 'cancelled') {
            msgEl.className = 'terra-dd-error';
            msgEl.textContent = res.message || ('Picker ' + res.status);
          }
        } catch (e) {
          msgEl.className = 'terra-dd-error';
          msgEl.textContent = 'Error: ' + e;
        }
        pickBtn.disabled = false;
        skipBtn.disabled = false;
        pickBtn.textContent = 'Choose folder\u2026';
      });

      skipBtn.addEventListener('click', async () => {
        skipBtn.disabled = true;
        pickBtn.disabled = true;
        try {
          await core.call('terra_dismiss_data_dir_prompt', {});
        } catch (e) {
          core.log('terra-ui', 'dismiss prompt error: ' + e);
        }
        closeSetup();
      });

      setupOverlay = h('div', { class: 'terra-overlay' },
        h('div', { class: 'terra-modal', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Terra data folder' },
          h('div', { class: 'terra-head' }, h('h3', {}, 'Terra data folder')),
          bodyEl,
          footEl
        )
      );
      document.body.append(setupOverlay);
    } catch (e) {
      core.log('terra-ui', 'data dir prompt check failed: ' + e);
    }
  }

  core.bus.on('terra:open-addons', (e) => openModal(e)); // addons can open their settings (terra.settings.open)
  setInterval(placeCog, 500); // the page re-renders its header now and then
  ensureCss(); // the cog needs its own styling from first paint (previously only attached when opening the modal)
  placeCog();
  if (document.body) {
    maybePromptDataDir();
  } else {
    document.addEventListener('DOMContentLoaded', maybePromptDataDir, { once: true });
  }
})();
