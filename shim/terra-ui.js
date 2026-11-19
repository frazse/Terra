// Terra's own UI: a cog in Biketerra's header (plus a subtle floating cog on /ride* & /spectate/* and Alt+T shortcut)
// that opens the addons modal. Built in (not an addon) so you can always get back to it, even with every addon off.
(() => {
  if (window.top !== window || window.__terraUi) return;
  const core = window.__terraCore, addons = window.__terraAddons;
  if (!core || !addons) return;
  window.__terraUi = true;

  const CSS = `
.terra-overlay { position: fixed; inset: 0; z-index: 2147483647; display: flex; align-items: center; justify-content: center;
  background: rgba(0,0,0,.65); font: 14px system-ui, sans-serif; }
.terra-modal { width: min(640px, calc(100vw - 28px)); max-height: calc(100vh - 48px); display: flex; flex-direction: column;
  background: #15151a; color: #eee; border-radius: 14px; box-shadow: 0 20px 60px rgba(0,0,0,.6); overflow: hidden; }
.terra-head { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 14px 20px; border-bottom: 1px solid #25252d; }
.terra-head-title { display: flex; align-items: center; gap: 10px; min-width: 0; }
.terra-head h3 { margin: 0; font-size: 17px; color: #fff; }
.terra-head-actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.terra-x { background: none; border: 0; color: #999; font-size: 24px; line-height: 1; cursor: pointer; padding: 0 4px; }
.terra-x:hover { color: #fff; }
.terra-toolbar { padding: 10px 20px; border-bottom: 1px solid #25252d; display: flex; flex-direction: column; gap: 8px; background: #191920; }
.terra-search-row { display: flex; gap: 8px; align-items: center; }
.terra-search { flex: 1; min-width: 0; padding: 7px 10px; border-radius: 7px; font: inherit; font-size: 13px;
  background: #23232c !important; color: #eee !important; border: 1px solid #343440 !important; }
.terra-chips { display: flex; gap: 6px; flex-wrap: wrap; }
.terra-chip { background: #24242e; color: #b5b5c3; border: 1px solid #32323e; border-radius: 999px; padding: 3px 10px; font: inherit; font-size: 12px; cursor: pointer; }
.terra-chip:hover { color: #fff; border-color: #484858; }
.terra-chip.active { background: #ff6b57; color: #fff; border-color: #ff6b57; font-weight: 600; }
.terra-banner { margin: 8px 20px 0; padding: 8px 12px; border-radius: 8px; font-size: 12px; display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.terra-banner.ok { background: #163022; border: 1px solid #255a3b; color: #8be8ac; }
.terra-banner.err { background: #361919; border: 1px solid #6b2626; color: #fca5a5; }
.terra-catalog { padding: 12px 20px; border-bottom: 1px solid #25252d; background: #121217; max-height: 280px; overflow: auto; }
.terra-catalog-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-bottom: 8px; }
.terra-catalog-head strong { font-size: 13px; color: #fff; }
.terra-cat-item { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 8px 0; border-top: 1px solid #22222b; }
.terra-cat-item:first-of-type { border-top: 0; }
.terra-body { padding: 4px 20px; overflow: auto; flex: 1; min-height: 120px; }
.terra-item { border-bottom: 1px solid #25252d; }
.terra-item:last-child { border-bottom: 0; }
.terra-row { display: flex; gap: 14px; align-items: center; padding: 13px 0; }
.terra-actions-row { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; margin-top: 8px; }
.terra-link { background: none; border: 0; padding: 0; color: #8fa3b8; font: inherit; font-size: 12px; cursor: pointer; text-decoration: underline; }
.terra-link:hover { color: #fff; }
.terra-link-danger { color: #e07b7b; }
.terra-link-danger:hover { color: #fca5a5; }
.terra-settings { padding: 0 0 16px; }
.terra-settings-foot { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; padding-top: 10px; border-top: 1px dashed #2d2d37; }
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
.terra-name { font-weight: 600; color: #fff; display: flex; align-items: center; flex-wrap: wrap; gap: 6px; }
.terra-ver { color: #888; font-weight: 400; font-size: 12px; }
.terra-badge { font-size: 10px; font-weight: 600; padding: 1px 6px; border-radius: 999px; background: #262632; color: #9ea8b8; border: 1px solid #353545; text-transform: uppercase; letter-spacing: .03em; }
.terra-badge.user { background: #1e293b; color: #7dd3fc; border-color: #334155; }
.terra-dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: #555; flex: none; }
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
.terra-ble { padding: 10px 20px; border-top: 1px solid #25252d; font-size: 12px; color: #bbb; background: #131318; }
.terra-ble summary { cursor: pointer; font-weight: 600; color: #ddd; display: flex; align-items: center; justify-content: space-between; list-style: none; }
.terra-ble summary::-webkit-details-marker { display: none; }
.terra-ble-list { margin-top: 8px; display: flex; flex-direction: column; gap: 6px; max-height: 150px; overflow: auto; }
.terra-ble-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 6px 8px; border-radius: 7px; background: #1c1c24; }
.terra-ble-meta { min-width: 0; flex: 1; }
.terra-ble-name { font-weight: 600; color: #eee; display: flex; align-items: center; gap: 6px; }
.terra-ble-id { font-family: monospace; font-size: 11px; color: #7c7c8c; overflow: hidden; text-overflow: ellipsis; }
.terra-ble-btns { display: flex; gap: 6px; flex: none; }
.terra-foot { display: flex; flex-direction: column; gap: 8px; padding: 10px 20px 14px; border-top: 1px solid #25252d; color: #888; font-size: 12px; }
.terra-foot-dirs { min-width: 0; word-break: break-all; }
.terra-foot-bottom { display: flex; align-items: center; justify-content: space-between; width: 100%; gap: 8px; }
.terra-foot-ver { color: #aaa; font-weight: 600; font-size: 12px; }
.terra-btn { flex: none; background: #2a2a33; color: #eee; border: 0; border-radius: 6px; padding: 6px 12px; font: inherit; font-size: 12px; cursor: pointer; }
.terra-btn:hover { background: #363642; }
.terra-btn-sm { padding: 4px 9px; font-size: 11px; }
.terra-cog { display: inline-flex; align-items: center; padding: 0 6px; margin: 0 2px; cursor: pointer; }
/* .nav-profile/.header prefix (specificity 0,2,0) so the page's own button rules can't re-apply a background */
.nav-profile .terra-cog, header .terra-cog { background: transparent; border: 0; color: inherit; }
.terra-cog:hover { opacity: .75; }
.action-button.terra-cog-action { display: inline-flex; align-items: center; justify-content: center; color: #d4d4d8; cursor: pointer; transition: color .15s; }
.action-button.terra-cog-action:hover { color: #ffffff; }
.action-button.terra-cog-action svg.action-icon { width: 20px; height: 20px; display: block; pointer-events: none; }
.terra-cog-float { position: fixed; top: 12px; right: 12px; z-index: 2147483644; width: 34px; height: 34px; padding: 0;
  border: 1px solid #3a3a44; border-radius: 50%; background: rgba(21, 21, 26, 0.78); color: #d4d4dc;
  display: flex; align-items: center; justify-content: center; cursor: pointer; opacity: .55; transition: opacity .15s;
  box-shadow: 0 2px 8px rgba(0,0,0,.35); }
.terra-cog-float:hover { opacity: 1; color: #fff; border-color: #5a5a68; }
.terra-datadir { padding: 10px 20px; border-top: 1px solid #25252d; font-size: 12px; color: #888; }
.terra-datadir .terra-dd-path { font-family: monospace; font-size: 11px; color: #aaa; word-break: break-all; margin: 4px 0; }
.terra-datadir .terra-dd-hint { color: #e6a23c; margin-bottom: 6px; }
.terra-datadir .terra-dd-actions { display: flex; gap: 8px; margin-top: 8px; flex-wrap: wrap; }
.terra-dd-restart { color: #7ddc8a; margin-top: 6px; font-size: 12px; }
.terra-dd-error { color: #f87171; margin-top: 6px; font-size: 12px; }
.terra-btn-primary { flex: none; background: #ff6b57; color: #fff; border: 0; border-radius: 6px; padding: 6px 12px; font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; }
.terra-btn-primary:hover { background: #ff5740; }
.terra-btn[disabled], .terra-btn-primary[disabled] { opacity: .6; cursor: default; }
.terra-setup-body { padding: 16px 20px; color: #ccc; line-height: 1.5; }
.terra-setup-body p { margin: 0 0 10px; }
.terra-setup-body .terra-dd-path { font-family: monospace; font-size: 11px; color: #aaa; word-break: break-all; margin: 8px 0; }
.terra-setup-body .terra-dd-hint { color: #e6a23c; font-size: 12px; margin: 8px 0; }
.terra-setup-foot { display: flex; gap: 10px; justify-content: flex-end; flex-wrap: wrap; padding: 12px 20px 16px; border-top: 1px solid #25252d; }
.terra-btn-danger { background: #3a1a1a; border: 1px solid #6b2020; color: #e06060; border-radius: 6px; padding: 4px 10px; cursor: pointer; font-size: 12px; }
.terra-btn-danger:hover { background: #4a2020; }
.terra-fg-body { margin-top: 10px; display: flex; flex-direction: column; gap: 10px; max-height: 240px; overflow: auto; padding-right: 2px; }
.terra-fg-status { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: 11px; color: #b5b5c3; }
.terra-fg-row { display: flex; gap: 8px; align-items: center; }
.terra-fg-row .terra-input { flex: 1; min-width: 0; }
.terra-fg-range-row { display: flex; align-items: center; gap: 10px; }
.terra-fg-range { flex: 1; accent-color: #ff6b57; cursor: pointer; }
.terra-fg-val { min-width: 40px; text-align: right; font-variant-numeric: tabular-nums; font-weight: 600; color: #eee; font-size: 12px; }
.terra-fg-check { display: flex; align-items: center; gap: 8px; font-size: 12px; color: #ddd; cursor: pointer; }
.terra-fg-check input { width: auto; accent-color: #ff6b57; cursor: pointer; }
.terra-fg-restart { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 8px 10px; border-radius: 7px; background: #2d1e14; border: 1px solid #6b4423; color: #fbd38d; font-size: 12px; }
.terra-fg-warn { color: #f6ad55; font-size: 11px; }
.terra-fg-ok { color: #7ddc8a; font-size: 11px; }
canvas, svg, .leaflet-container, [class*="map"], [class*="elevation"] {
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
  let overlay = null, body = null, foot = null, bannerSlot = null, catalogBox = null, bleBox = null;
  let searchQuery = '';
  let activeCategory = 'All';
  let catalogOpen = false;
  let cachedList = null;

  const dots = new Map(); // addon id -> dot element
  const settingsToggles = new Map(); // addon id -> open/close its settings section
  const previews = new Map(); // addon id -> live preview slot element (only while its settings are open)
  const paintDots = () => { for (const [id, dot] of dots) dot.classList.toggle('on', addons.active.has(id)); };

  const firePreview = (id, slot) => core.bus.emit('settings:preview:' + id, { slot: slot || null });
  const colorOf = (v) => typeof v === 'string' && /^#[0-9a-f]{6}$/i.test(v.trim()) ? v.trim() : '#000000';

  function showBanner(msg, kind = 'ok') {
    if (!bannerSlot) return;
    bannerSlot.replaceChildren(
      h('div', { class: 'terra-banner ' + kind },
        h('span', {}, msg),
        h('button', { class: 'terra-x', type: 'button', onclick: () => bannerSlot.replaceChildren() }, '\u00d7'))
    );
  }

  function bytesToB64(bytes) {
    let bin = '';
    const chunk = 0x8000;
    for (let i = 0; i < bytes.length; i += chunk) {
      bin += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
    }
    return btoa(bin);
  }

  async function installZipFile(file) {
    if (!file) return;
    try {
      const buf = await file.arrayBuffer();
      const b64 = bytesToB64(new Uint8Array(buf));
      const installed = await core.call('addons_install_zip', { base64: b64, filename: file.name });
      const names = (installed || []).map((x) => `${x.name} v${x.version}`).join(', ');
      showBanner('Installed: ' + (names || file.name), 'ok');
      addons.refresh();
      await render(true);
      if (catalogOpen) await loadCatalog(false);
    } catch (e) {
      showBanner('Zip install failed: ' + e, 'err');
    }
  }

  function downloadJson(filename, obj) {
    const text = JSON.stringify(obj, null, 2);
    const blob = new Blob([text], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 2000);
  }

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
          core.bus.emit('settings:' + a.id, { key: def.key });
          const p = previews.get(a.id);
          if (p) firePreview(a.id, p);
        } catch (e) { status.textContent = 'error: ' + e; }
        setTimeout(() => { status.textContent = ''; }, 2500);
      };
      clearBtn.addEventListener('click', () => save(''));
      input.addEventListener('change', () => {
        if (def.type === 'checkbox') return save(input.checked);
        if (secret && input.value === '') return;
        return save(def.type === 'number' ? (input.value === '' ? '' : Number(input.value)) : input.value.trim());
      });
      box.append(h('div', { class: 'terra-field' },
        h('label', {}, def.label || def.key, status),
        input,
        secret ? clearBtn : '',
        def.description ? h('div', { class: 'terra-help' }, def.description) : ''));
    }

    const resetSettingsBtn = h('button', { class: 'terra-btn terra-btn-sm', type: 'button' }, 'Reset settings to defaults');
    resetSettingsBtn.addEventListener('click', async () => {
      if (!confirm(`Reset all cog settings for "${a.name}" back to defaults?`)) return;
      try {
        await core.call('addon_settings_reset', { id: a.id });
        core.bus.emit('settings:' + a.id, { reset: true });
        await fillSettings(box, a);
        showBanner(`Reset settings for ${a.name} to defaults.`, 'ok');
      } catch (e) {
        showBanner('Reset failed: ' + e, 'err');
      }
    });
    box.append(h('div', { class: 'terra-settings-foot' }, resetSettingsBtn));

    if (slot) { core.log('terra-ui', `firing preview for ${a.id}`); firePreview(a.id, slot); }
  }

  function hidePreviews() {
    for (const [id, p] of previews) { previews.delete(id); firePreview(id, null); }
  }

  async function loadCatalog(forceRefresh = false) {
    if (!catalogBox) return;
    catalogBox.hidden = false;
    catalogBox.replaceChildren(h('div', { class: 'terra-catalog-head' },
      h('strong', {}, 'GitHub Catalog (frazse/Terra-Addons)'),
      h('span', { class: 'terra-ver' }, 'Loading\u2026')));
    try {
      const items = await core.call('addons_catalog', { refresh: !!forceRefresh });
      const refreshBtn = h('button', { class: 'terra-btn terra-btn-sm', type: 'button' }, 'Refresh');
      refreshBtn.addEventListener('click', () => loadCatalog(true));
      const closeCatBtn = h('button', { class: 'terra-btn terra-btn-sm', type: 'button' }, 'Hide');
      closeCatBtn.addEventListener('click', () => { catalogOpen = false; catalogBox.hidden = true; });
      const head = h('div', { class: 'terra-catalog-head' },
        h('strong', {}, `GitHub Catalog (${items.length})`),
        h('div', { style: 'display:flex;gap:6px;' }, refreshBtn, closeCatBtn));
      catalogBox.replaceChildren(head);
      if (!items.length) {
        catalogBox.append(h('div', { class: 'terra-meta' }, 'No addons found in repository.'));
        return;
      }
      for (const item of items) {
        const isInstalled = !!item.installedVersion;
        const btnLabel = item.updateAvailable
          ? `Update (${item.installedVersion} \u2192 ${item.version})`
          : isInstalled ? `Reinstall v${item.version}` : `Install v${item.version}`;
        const btn = h('button', {
          class: (item.updateAvailable || !isInstalled) ? 'terra-btn-primary terra-btn-sm' : 'terra-btn terra-btn-sm',
          type: 'button',
        }, btnLabel);
        btn.addEventListener('click', async () => {
          btn.disabled = true;
          btn.textContent = 'Installing\u2026';
          try {
            const inst = await core.call('addons_install_github', { id: item.id });
            showBanner(`Installed ${inst.name} v${inst.version} from GitHub.`, 'ok');
            addons.refresh();
            await render(true);
            await loadCatalog(false);
          } catch (e) {
            showBanner(`Failed to install ${item.id}: ${e}`, 'err');
            btn.disabled = false;
            btn.textContent = btnLabel;
          }
        });
        catalogBox.append(h('div', { class: 'terra-cat-item' },
          h('div', { class: 'terra-info' },
            h('div', { class: 'terra-name' },
              item.name,
              h('span', { class: 'terra-ver' }, 'v' + item.version),
              h('span', { class: 'terra-badge' }, item.category || 'Other')),
            item.description ? h('div', { class: 'terra-desc' }, item.description) : ''),
          btn));
      }
    } catch (e) {
      const retry = h('button', { class: 'terra-btn terra-btn-sm', type: 'button' }, 'Retry');
      retry.addEventListener('click', () => loadCatalog(true));
      catalogBox.replaceChildren(
        h('div', { class: 'terra-catalog-head' },
          h('strong', {}, 'GitHub Catalog'),
          retry),
        h('div', { class: 'terra-dd-error' }, 'Failed to fetch catalog: ' + e)
      );
    }
  }

  async function renderBlePanel() {
    if (!bleBox) return;
    const listWrap = bleBox.querySelector('.terra-ble-list');
    const summaryBadge = bleBox.querySelector('.terra-ble-summary-count');
    if (!listWrap) return;
    try {
      const devs = await core.call('ble_devices_status', {});
      const connCount = devs.filter((d) => d.connected).length;
      if (summaryBadge) {
        summaryBadge.textContent = devs.length
          ? `${connCount} connected \u00b7 ${devs.length} total`
          : 'none';
      }
      listWrap.replaceChildren();
      if (!devs.length) {
        listWrap.append(h('div', { class: 'terra-meta' }, 'No remembered or connected Bluetooth devices.'));
        return;
      }
      for (const d of devs) {
        const btns = [];
        if (d.connected) {
          const discBtn = h('button', { class: 'terra-btn terra-btn-sm', type: 'button' }, 'Disconnect');
          discBtn.addEventListener('click', async () => {
            discBtn.disabled = true;
            try {
              await core.call('ble_drop_device', { id: d.id });
              await renderBlePanel();
            } catch (e) {
              showBanner('Disconnect failed: ' + e, 'err');
              discBtn.disabled = false;
            }
          });
          btns.push(discBtn);
        }
        if (d.remembered) {
          const forgetBtn = h('button', { class: 'terra-btn-danger terra-btn-sm', type: 'button' }, 'Forget');
          forgetBtn.addEventListener('click', async () => {
            if (!confirm(`Forget "${d.name || d.id}"? You will need to pair it again.`)) return;
            forgetBtn.disabled = true;
            try {
              await core.call('ble_forget', { id: d.id });
              await renderBlePanel();
            } catch (e) {
              showBanner('Forget failed: ' + e, 'err');
              forgetBtn.disabled = false;
            }
          });
          btns.push(forgetBtn);
        }
        listWrap.append(h('div', { class: 'terra-ble-row' },
          h('div', { class: 'terra-ble-meta' },
            h('div', { class: 'terra-ble-name' },
              h('span', { class: 'terra-dot' + (d.connected ? ' on' : '') }),
              d.name || 'Unnamed device',
              d.owner ? h('span', { class: 'terra-badge' }, d.owner) : ''),
            h('div', { class: 'terra-ble-id' }, d.id)),
          h('div', { class: 'terra-ble-btns' }, ...btns)));
      }
    } catch (e) {
      if (summaryBadge) summaryBadge.textContent = 'unavailable';
      listWrap.replaceChildren(h('div', { class: 'terra-meta' }, 'Bluetooth status unavailable: ' + e));
    }
  }

  function filteredAddons(all) {
    const q = searchQuery.trim().toLowerCase();
    return all.filter((a) => {
      if (activeCategory === 'Enabled' && !a.enabled) return false;
      if (activeCategory !== 'All' && activeCategory !== 'Enabled' && (a.category || 'Other') !== activeCategory) return false;
      if (!q) return true;
      const hay = `${a.id} ${a.name} ${a.description || ''} ${a.category || ''}`.toLowerCase();
      return hay.includes(q);
    });
  }

  async function render(forceFetch = true) {
    let res = cachedList;
    if (forceFetch || !res) {
      try {
        res = await core.call('addons_list');
        cachedList = res;
      } catch (e) {
        body.textContent = 'Could not load addons: ' + e;
        return;
      }
    }
    dots.clear();
    settingsToggles.clear();
    hidePreviews();
    body.replaceChildren();

    const list = filteredAddons(res.addons);
    if (!res.addons.length) {
      body.append(h('div', { class: 'terra-empty' }, 'No addons found. Put addon folders in:', ...res.dirs.map((d) => h('code', {}, d))));
    } else if (!list.length) {
      body.append(h('div', { class: 'terra-empty' }, 'No addons match the current search or filter.'));
    }

    for (const a of list) {
      const dot = h('span', { class: 'terra-dot' });
      dots.set(a.id, dot);
      const input = h('input', { type: 'checkbox', 'aria-label': 'Enable ' + a.name });
      input.checked = a.enabled;
      input.addEventListener('change', async () => {
        try {
          await core.call('addons_set_enabled', { id: a.id, enabled: input.checked });
          a.enabled = input.checked;
          addons.refresh();
        } catch (e) {
          input.checked = !input.checked;
          core.log('terra-ui', 'toggle failed: ' + e);
        }
        setTimeout(paintDots, 600);
        setTimeout(paintDots, 1800);
      });

      const box = h('div', { class: 'terra-settings', hidden: '' });
      const actionItems = [];
      if (a.settings && a.settings.length) {
        const toggleBtn = h('button', { class: 'terra-link', type: 'button' }, 'Settings \u25be');
        const toggle = async (force) => {
          const open = force === undefined ? box.hidden : force;
          if (open) { await fillSettings(box, a); box.hidden = false; toggleBtn.textContent = 'Settings \u25b4'; }
          else if (!box.hidden) {
            box.hidden = true;
            toggleBtn.textContent = 'Settings \u25be';
            const p = previews.get(a.id);
            if (p) { previews.delete(a.id); firePreview(a.id, null); }
          }
        };
        toggleBtn.addEventListener('click', () => toggle());
        settingsToggles.set(a.id, toggle);
        actionItems.push(toggleBtn);
      }

      const exportBtn = h('button', { class: 'terra-link', type: 'button' }, 'Export JSON');
      exportBtn.addEventListener('click', async () => {
        try {
          const backup = await core.call('addon_settings_export', { id: a.id });
          downloadJson(`${a.id}-backup.json`, backup);
          showBanner(`Exported settings & storage for ${a.name}.`, 'ok');
        } catch (e) {
          showBanner('Export failed: ' + e, 'err');
        }
      });
      actionItems.push(exportBtn);

      const importBtn = h('button', { class: 'terra-link', type: 'button' }, 'Import JSON');
      importBtn.addEventListener('click', () => {
        const picker = document.createElement('input');
        picker.type = 'file';
        picker.accept = '.json,application/json';
        picker.addEventListener('change', async () => {
          const file = picker.files && picker.files[0];
          if (!file) return;
          try {
            const payload = JSON.parse(await file.text());
            const r = await core.call('addon_settings_import', { id: a.id, payload });
            core.bus.emit('settings:' + a.id, { imported: true });
            if (!box.hidden) await fillSettings(box, a);
            showBanner(`Imported ${r.settingsCount} setting(s) & ${r.storageCount} storage key(s) for ${a.name}.`, 'ok');
          } catch (e) {
            showBanner('Import failed: ' + e, 'err');
          }
        });
        picker.click();
      });
      actionItems.push(importBtn);

      if (a.userInstalled) {
        const uninstBtn = h('button', { class: 'terra-link terra-link-danger', type: 'button' }, a.bundled ? 'Remove override' : 'Uninstall');
        uninstBtn.addEventListener('click', async () => {
          const msg = a.bundled
            ? `Remove user-installed copy of "${a.name}"? The bundled version will take over again.`
            : `Uninstall "${a.name}" (${a.id})?`;
          if (!confirm(msg)) return;
          try {
            await core.call('addons_uninstall', { id: a.id });
            showBanner((a.bundled ? 'Removed user override for ' : 'Uninstalled ') + a.name + '.', 'ok');
            addons.refresh();
            await render(true);
            if (catalogOpen) await loadCatalog(false);
          } catch (e) {
            showBanner('Uninstall failed: ' + e, 'err');
          }
        });
        actionItems.push(uninstBtn);
      }

      const badges = [];
      if (a.category) badges.push(h('span', { class: 'terra-badge' }, a.category));
      if (a.userInstalled) badges.push(h('span', { class: 'terra-badge user', title: a.bundled ? 'Overrides bundled copy' : 'User-installed' }, a.bundled ? 'Override' : 'Installed'));

      body.append(h('div', { class: 'terra-item' },
        h('div', { class: 'terra-row' },
          h('div', { class: 'terra-info' },
            h('div', { class: 'terra-name' }, dot, a.name, h('span', { class: 'terra-ver' }, 'v' + a.version), ...badges),
            a.description ? h('div', { class: 'terra-desc' }, a.description) : '',
            h('div', { class: 'terra-meta' }, a.id + ' \u00b7 ' + a.matches.join(', ') + (a.defaultEnabled ? '' : ' \u00b7 off by default')),
            h('div', { class: 'terra-actions-row' }, ...actionItems)),
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
    bannerSlot = h('div', {});
    catalogBox = h('div', { class: 'terra-catalog', hidden: '' });
    catalogOpen = false;

    const zipInput = h('input', { type: 'file', accept: '.zip,application/zip', hidden: '' });
    zipInput.addEventListener('change', () => {
      const file = zipInput.files && zipInput.files[0];
      zipInput.value = '';
      installZipFile(file);
    });

    const catalogBtn = h('button', { class: 'terra-btn terra-btn-sm', type: 'button' }, 'GitHub Catalog');
    catalogBtn.addEventListener('click', () => {
      catalogOpen = !catalogOpen;
      if (catalogOpen) loadCatalog(false);
      else catalogBox.hidden = true;
    });

    const uploadBtn = h('button', { class: 'terra-btn terra-btn-sm', type: 'button', title: 'Install an addon from a .zip file' }, 'Install .zip');
    uploadBtn.addEventListener('click', () => zipInput.click());

    const searchInput = h('input', {
      class: 'terra-search', type: 'search', placeholder: 'Search addons\u2026', value: searchQuery,
    });
    searchInput.addEventListener('input', () => {
      searchQuery = searchInput.value;
      render(false);
    });

    const cats = ['All', 'Enabled', 'HUD', 'Camera', 'Hardware', 'Export', 'Other'];
    const chipsRow = h('div', { class: 'terra-chips' });
    const paintChips = () => {
      chipsRow.replaceChildren(...cats.map((c) => {
        const b = h('button', {
          class: 'terra-chip' + (activeCategory === c ? ' active' : ''),
          type: 'button',
        }, c);
        b.addEventListener('click', () => {
          activeCategory = c;
          paintChips();
          render(false);
        });
        return b;
      }));
    };
    paintChips();

    const toolbar = h('div', { class: 'terra-toolbar' },
      h('div', { class: 'terra-search-row' }, searchInput),
      chipsRow);

    // Bluetooth devices section (collapsible)
    const bleRefreshBtn = h('button', { class: 'terra-btn terra-btn-sm', type: 'button' }, 'Refresh');
    bleRefreshBtn.addEventListener('click', (e) => { e.preventDefault(); renderBlePanel(); });
    const bleDropAllBtn = h('button', { class: 'terra-btn terra-btn-sm', type: 'button' }, 'Disconnect all');
    bleDropAllBtn.addEventListener('click', async (e) => {
      e.preventDefault();
      bleDropAllBtn.disabled = true;
      try {
        await core.call('ble_drop_all', {});
        await renderBlePanel();
        showBanner('Disconnected active Bluetooth devices.', 'ok');
      } catch (err) {
        showBanner('Disconnect all failed: ' + err, 'err');
      }
      bleDropAllBtn.disabled = false;
    });

    bleBox = h('details', { class: 'terra-ble' },
      h('summary', {},
        h('span', {}, 'Bluetooth devices ', h('span', { class: 'terra-ble-summary-count terra-ver' }, '')),
        h('span', { style: 'display:flex;gap:6px;' }, bleRefreshBtn, bleDropAllBtn)),
      h('div', { class: 'terra-ble-list' }, 'Loading\u2026'));
    bleBox.addEventListener('toggle', () => { if (bleBox.open) renderBlePanel(); });

    // Frame Generation section (Linux Electron desktop only; shown if host supports framegen_get)
    const fgSummaryBadge = h('span', { class: 'terra-ver' }, '');
    const fgBody = h('div', { class: 'terra-fg-body' });
    const fgBox = h('details', { class: 'terra-ble terra-fg', hidden: '' },
      h('summary', {},
        h('span', {}, 'Frame Generation (Experimental) ', fgSummaryBadge)),
      fgBody);

    // Data folder section
    const ddSection = h('div', { class: 'terra-datadir' }, h('strong', {}, 'Data folder'));
    foot = h('div', { class: 'terra-foot' },
      ddSection,
      h('div', { class: 'terra-foot-dirs' }),
      h('div', { class: 'terra-foot-bottom' },
        h('div', { class: 'terra-foot-ver' }, 'Terra'),
        h('button', { class: 'terra-btn', type: 'button', onclick: () => location.reload() }, 'Reload page')
      )
    );

    const modal = h('div', { class: 'terra-modal', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Terra addons' },
      h('div', { class: 'terra-head' },
        h('div', { class: 'terra-head-title' }, h('h3', {}, 'Terra addons')),
        h('div', { class: 'terra-head-actions' },
          catalogBtn,
          uploadBtn,
          zipInput,
          h('button', { class: 'terra-x', type: 'button', 'aria-label': 'Close', onclick: closeModal }, '\u00d7'))),
      toolbar,
      bannerSlot,
      catalogBox,
      body,
      bleBox,
      fgBox,
      foot);

    // Drag & drop .zip directly onto the modal
    modal.addEventListener('dragover', (e) => { e.preventDefault(); });
    modal.addEventListener('drop', (e) => {
      e.preventDefault();
      const file = e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files[0];
      if (file && file.name.toLowerCase().endsWith('.zip')) installZipFile(file);
    });

    overlay = h('div', { class: 'terra-overlay' }, modal);
    overlay.addEventListener('click', (e) => { if (e.target === overlay) closeModal(); });
    addEventListener('keydown', onKey, true);
    document.body.append(overlay);

    renderBlePanel();

    // Populate Frame Generation panel on Linux Electron desktop
    if (!navigator.userAgent.includes('Android')) {
      (async () => {
        const modeLabels = {
          off: 'Off',
          '2x': '2x (Fixed)',
          '3x': '3x (Fixed)',
          '4x': '4x (Fixed)',
          adaptive60: 'Adaptive 60 FPS',
          adaptive120: 'Adaptive 120 FPS',
        };
        const shortModeLabels = {
          off: 'Off',
          '2x': '2x',
          '3x': '3x',
          '4x': '4x',
          adaptive60: 'Adaptive 60',
          adaptive120: 'Adaptive 120',
        };

        const renderFramegen = (st) => {
          if (!st || !st.supported) return;
          fgBox.hidden = false;
          const cfg = st.config || { mode: 'off', dllPath: '', flowScale: 85, performanceMode: false, rideOnly: true };
          const summaryParts = [shortModeLabels[cfg.mode] || cfg.mode];
          if (cfg.mode !== 'off') summaryParts.push(`${cfg.flowScale}%`);
          if (st.restartRequired) summaryParts.push('restart required');
          fgSummaryBadge.textContent = summaryParts.join(' \u00b7 ');

          const saveFeedback = h('span', { class: 'terra-status' });
          const saveFg = async (patch) => {
            try {
              const next = await core.call('framegen_set', patch);
              core.bus.emit('terra:framegen-changed', next);
              renderFramegen(next);
              const fb = fgBody.querySelector('.terra-fg-saved');
              if (fb) {
                fb.textContent = 'saved';
                setTimeout(() => { fb.textContent = ''; }, 2000);
              }
            } catch (e) {
              showBanner('Frame generation save failed: ' + e, 'err');
            }
          };

          const dllInput = h('input', {
            class: 'terra-input',
            type: 'text',
            value: cfg.dllPath || '',
            placeholder: '/path/to/Lossless.dll or lsfg-vk.dll',
            autocomplete: 'off',
            spellcheck: 'false',
          });
          dllInput.addEventListener('change', () => saveFg({ dllPath: dllInput.value.trim() }));

          const pickDllBtn = h('button', { class: 'terra-btn terra-btn-sm', type: 'button' }, 'Browse\u2026');
          pickDllBtn.addEventListener('click', async () => {
            pickDllBtn.disabled = true;
            try {
              const res = await core.call('framegen_pick_dll', {});
              if (res && !res.cancelled) {
                core.bus.emit('terra:framegen-changed', res);
                renderFramegen(res);
              }
            } catch (e) {
              showBanner('DLL picker failed: ' + e, 'err');
            }
            pickDllBtn.disabled = false;
          });

          const modeSelect = h('select', { class: 'terra-input' });
          for (const [val, label] of Object.entries(modeLabels)) {
            modeSelect.append(h('option', { value: val }, label));
          }
          modeSelect.value = cfg.mode;
          modeSelect.addEventListener('change', () => saveFg({ mode: modeSelect.value }));

          const flowValEl = h('span', { class: 'terra-fg-val' }, `${cfg.flowScale}%`);
          const flowSlider = h('input', {
            class: 'terra-fg-range',
            type: 'range',
            min: '0',
            max: '100',
            step: '5',
          });
          flowSlider.value = String(cfg.flowScale);
          flowSlider.addEventListener('input', () => {
            flowValEl.textContent = `${flowSlider.value}%`;
          });
          flowSlider.addEventListener('change', () => saveFg({ flowScale: Number(flowSlider.value) }));

          const perfCheckbox = h('input', { type: 'checkbox' });
          perfCheckbox.checked = !!cfg.performanceMode;
          perfCheckbox.addEventListener('change', () => saveFg({ performanceMode: perfCheckbox.checked }));

          const rideOnlyCheckbox = h('input', { type: 'checkbox' });
          rideOnlyCheckbox.checked = cfg.rideOnly !== false;
          rideOnlyCheckbox.addEventListener('change', () => saveFg({ rideOnly: rideOnlyCheckbox.checked }));

          const restartBtn = h('button', { class: 'terra-btn-primary terra-btn-sm', type: 'button' }, 'Restart Terra');
          restartBtn.addEventListener('click', async () => {
            restartBtn.disabled = true;
            restartBtn.textContent = 'Restarting\u2026';
            try {
              await core.call('framegen_relaunch', {});
            } catch (e) {
              showBanner('Restart failed: ' + e, 'err');
              restartBtn.disabled = false;
              restartBtn.textContent = 'Restart Terra';
            }
          });

          const dllStatusEl = !cfg.dllPath
            ? h('div', { class: 'terra-fg-warn' }, 'No Lossless.dll / lsfg-vk.dll selected yet.')
            : !st.dllExists
              ? h('div', { class: 'terra-fg-warn' }, 'DLL file not found at this path.')
              : '';

          saveFeedback.classList.add('terra-fg-saved');

          const statusText = st.activeNow
            ? 'Frame gen active on this page'
            : st.bootActive
              ? (cfg.mode !== 'off' && cfg.rideOnly !== false && !st.onRidePage
                  ? 'Frame gen armed (active on /ride & /spectate)'
                  : 'Frame gen paused this session')
              : 'Frame gen inactive this session';

          fgBody.replaceChildren(
            h('div', { class: 'terra-fg-status' },
              h('span', { class: 'terra-dot' + (st.bootActive && cfg.mode !== 'off' ? ' on' : '') }),
              h('span', {}, statusText),
              h('span', {}, '\u00b7'),
              h('span', { class: st.layerInstalled ? 'terra-fg-ok' : 'terra-fg-warn' },
                st.layerInstalled ? 'lsfg-vk layer detected' : 'lsfg-vk layer not detected'),
              saveFeedback),
            st.restartRequired
              ? h('div', { class: 'terra-fg-restart' },
                  h('span', {}, 'Restart Terra to apply frame generation / display changes.'),
                  restartBtn)
              : '',
            h('div', { class: 'terra-field', style: 'margin-top:0' },
              h('label', {}, 'Lossless Scaling DLL (Lossless.dll / lsfg-vk.dll)'),
              h('div', { class: 'terra-fg-row' }, dllInput, pickDllBtn),
              dllStatusEl),
            h('div', { class: 'terra-field', style: 'margin-top:0' },
              h('label', {}, 'Mode'),
              modeSelect,
              h('div', { class: 'terra-help' },
                '2x / 3x / 4x run via lsfg-vk 2.0 on XWayland + Vulkan. Adaptive 60 / 120 sets experimental LSFGVK_ADAPTIVE env vars (falls back to 2x on standard lsfg-vk 2.0).')),
            h('div', { class: 'terra-field', style: 'margin-top:0' },
              h('label', {}, 'Flow scale'),
              h('div', { class: 'terra-fg-range-row' }, flowSlider, flowValEl),
              h('div', { class: 'terra-help' },
                '0\u2013100% motion estimation scale. lsfg-vk enforces a 25% minimum (values below 25% clamp to 25%).')),
            h('label', { class: 'terra-fg-check' },
              perfCheckbox,
              h('span', {}, 'Performance mode (faster, slightly lower quality)')),
            h('label', { class: 'terra-fg-check' },
              rideOnlyCheckbox,
              h('span', {}, 'Only active on ride & spectate pages'))
          );
        };

        try {
          const st = await core.call('framegen_get', {});
          renderFramegen(st);
          if (st && st.restartRequired) fgBox.open = true;
        } catch {
          /* Non-Electron hosts (Tauri / Android) do not implement framegen_get */
        }
      })();
    }

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
        ddSection.append(h('div', { class: 'terra-dd-path' }, 'data folder feature unavailable'));
      }
    })();

    await render(true);
    if (opts && opts.expand) settingsToggles.get(opts.expand)?.(true);
  }

  /* ---------- cog in Biketerra's header, in-ride .action-buttons, or floating fallback ---------- */
  const GEAR = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">' +
    '<circle cx="12" cy="12" r="3"></circle>' +
    '<path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path></svg>';
  const GEAR_ACTION = GEAR
    .replace('width="18" height="18"', 'class="action-icon"')
    .replace('stroke-width="1.5"', 'stroke-width="1.35"');
  let cog = null, actionCog = null, floatCog = null;
  function placeCog() {
    const actionButtons = document.querySelector('.action-buttons');
    if (actionButtons) {
      if (floatCog && floatCog.parentNode) floatCog.remove();
      if (!actionCog) {
        actionCog = h('div', {
          class: 'action-button terra-cog-action',
          role: 'button',
          tabindex: '-1',
          'aria-label': 'Terra addons',
          title: 'Terra addons (Alt+T)',
        });
        actionCog.innerHTML = GEAR_ACTION;
        actionCog.addEventListener('click', (e) => {
          e.stopPropagation();
          openModal();
        });
      }
      if (actionCog.parentNode !== actionButtons) actionButtons.append(actionCog);
      const refIcon = actionButtons.querySelector('.action-button:not(.terra-cog-action) .action-icon');
      const svg = actionCog.querySelector('svg');
      if (refIcon && svg) {
        const r = refIcon.getBoundingClientRect();
        const sz = Math.round(r.height);
        if (sz >= 14 && sz <= 48) {
          svg.style.width = sz + 'px';
          svg.style.height = sz + 'px';
        }
      }
      return;
    } else if (actionCog && actionCog.parentNode) {
      actionCog.remove();
    }

    const help = document.querySelector('.nav-profile a[href="/help-center"]') ||
      Array.from(document.querySelectorAll('.nav-profile a')).find((a) => a.textContent.trim().toLowerCase() === 'help');
    const anchor = help ||
      document.querySelector('.nav-profile .theme-toggle') ||
      document.querySelector('.theme-toggle');

    if (anchor) {
      if (floatCog && floatCog.parentNode) floatCog.remove();
      if (!cog) {
        cog = h('button', { type: 'button', 'aria-label': 'Terra addons', title: 'Terra addons (Alt+T)' });
        cog.innerHTML = GEAR;
        cog.addEventListener('click', () => openModal());
      }
      if (cog.className !== 'terra-cog') cog.className = 'terra-cog';
      if (anchor.nextElementSibling !== cog) anchor.after(cog);
      return;
    }

    const onRideLike = location.pathname.startsWith('/ride') || location.pathname.startsWith('/spectate');
    if (onRideLike && document.body) {
      if (!floatCog) {
        floatCog = h('button', { type: 'button', class: 'terra-cog-float', 'aria-label': 'Terra addons', title: 'Terra addons (Alt+T)' });
        floatCog.innerHTML = GEAR;
        floatCog.addEventListener('click', () => openModal());
      }
      if (!floatCog.parentNode) document.body.append(floatCog);
    } else if (floatCog && floatCog.parentNode) {
      floatCog.remove();
    }
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

  // Global Alt+T shortcut to toggle the Terra addons modal
  addEventListener('keydown', (e) => {
    if (e.altKey && !e.ctrlKey && !e.metaKey && e.key.toLowerCase() === 't') {
      e.preventDefault();
      if (overlay) closeModal();
      else openModal();
    }
  }, true);

  core.bus.on('terra:open-addons', (e) => openModal(e));
  setInterval(placeCog, 500);
  ensureCss();
  placeCog();
  if (document.body) {
    maybePromptDataDir();
  } else {
    document.addEventListener('DOMContentLoaded', maybePromptDataDir, { once: true });
  }
})();
