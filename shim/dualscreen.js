// Android Dual-Screen Presentation bridge (Screen 1 <-> Screen 2).
(() => {
  if (window.top !== window || window.__terraDualScreen) return;
  if (!navigator.userAgent.includes('Android')) return;
  const core = window.__terraCore;
  if (!core) return;

  const CSS = `
html.terra-dualscreen-main .hud,
html.terra-dualscreen-main .ride-menu,
html.terra-dualscreen-main .menu-backing,
html.terra-dualscreen-main .load-overlay,
html.terra-dualscreen-main .cinecam-warn,
html.terra-dualscreen-main .spectating-name,
html.terra-dualscreen-main .terra-cog-float,
html.terra-dualscreen-main .terra-pip-float,
html.terra-dualscreen-main #terra-fps-stats,
html.terra-dualscreen-main #fps-monitor-hud,
html.terra-dualscreen-main .tr-cp-root,
html.terra-dualscreen-main #terra-gear-hud,
html.terra-dualscreen-main #tvc-hud,
html.terra-dualscreen-main #btz-console-hud,
html.terra-dualscreen-main #btz-ring,
html.terra-dualscreen-main #terra-gfx-split-handle,
html.terra-dualscreen-main #terra-export-mode-panel,
html.terra-dualscreen-main .terra-overlay {
  opacity: 0 !important;
  pointer-events: none !important;
}
html.terra-dualscreen-main.terra-dualscreen-hide-nameplates .nameplates-ov {
  opacity: 0 !important;
  pointer-events: none !important;
}
html.terra-dualscreen-mirror .hud .zone-effort {
  width: calc(var(--terra-mir-graph-w, 210px) + 16px) !important;
  max-width: none !important;
  overflow: visible !important;
}
html.terra-dualscreen-mirror .hud .zone-effort .stats-grid {
  width: var(--terra-mir-graph-w, 210px) !important;
  min-width: var(--terra-mir-graph-w, 210px) !important;
  max-width: var(--terra-mir-graph-w, 210px) !important;
  grid-template-columns: minmax(0, 1fr) !important;
}
html.terra-dualscreen-mirror .hud .zone-effort .stat-cell:has(canvas.terra-dark-graph) {
  grid-column: 1 / -1 !important;
  width: var(--terra-mir-graph-w, 210px) !important;
  min-width: var(--terra-mir-graph-w, 210px) !important;
  max-width: var(--terra-mir-graph-w, 210px) !important;
}
html.terra-dualscreen-mirror .hud .zone-effort .stat-cell .stat-value {
  font-size: var(--terra-mir-graph-font, 14.48px) !important;
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
      el.setAttribute('data-terra-dual-css', '1');
      el.textContent = CSS;
      (document.head || document.documentElement).append(el);
    }
  };

  const isRideLike = () =>
    location.pathname.startsWith('/ride') || location.pathname.startsWith('/spectate');

  let dualState = {
    supported: false,
    hasSecondaryDisplay: false,
    displayId: -1,
    displayName: '',
    displayWidth: 0,
    displayHeight: 0,
    enabled: true,
    mode: 'cockpit',
    keepNameplates: true,
    inRide: isRideLike(),
    active: false,
  };

  let wasMirrorApplied = false;
  function applyDualBodyClasses() {
    ensureCss();
    const isActive = !!(dualState.active && isRideLike());
    const hideNp = isActive && !dualState.keepNameplates;
    const isMirror = isActive && dualState.mode === 'mirror';
    for (const el of [document.documentElement, document.body]) {
      if (!el) continue;
      el.classList.toggle('terra-dualscreen-main', isActive);
      el.classList.toggle('terra-dualscreen-mirror', isMirror);
      el.classList.toggle('terra-dualscreen-hide-nameplates', hideNp);
    }
    if (isMirror !== wasMirrorApplied) {
      wasMirrorApplied = isMirror;
      graphSnapCache.clear();
      lastGraphSnapAt = 0;
      if (isMirror) {
        setTimeout(() => pushNow(true), 260);
        setTimeout(() => pushNow(true), 600);
      }
    }
  }

  let pushTimer = null;
  let lastPushAt = 0;
  let pushBusy = false;
  let lastMapSnapAt = 0;
  let lastMapSnapUrl = '';
  let lastGraphSnapAt = 0;
  const graphSnapCache = new Map();

  function schedulePush(delayMs = 200) {
    if (!dualState.active || !isRideLike()) return;
    if (pushTimer) return;
    pushTimer = setTimeout(() => {
      pushTimer = null;
      pushNow();
    }, delayMs);
  }

  window.__terraSetDualScreenState = (st) => {
    if (!st || typeof st !== 'object') return;
    const wasActive = dualState.active;
    const prevMode = dualState.mode;
    dualState = { ...dualState, ...st };
    applyDualBodyClasses();
    core.bus.emit('terra:dualscreen-changed', dualState);
    if (dualState.active && (!wasActive || prevMode !== dualState.mode)) {
      pushNow(true);
    }
  };

  async function syncStatus() {
    try {
      const st = await core.call('terra_dualscreen_status', {});
      if (st && typeof st === 'object') {
        dualState = { ...dualState, ...st };
        if (st.supported && st.inRide !== isRideLike()) {
          const next = await core.call('terra_dualscreen_configure', { inRide: isRideLike() });
          if (next && typeof next === 'object') dualState = { ...dualState, ...next };
        }
        applyDualBodyClasses();
        core.bus.emit('terra:dualscreen-changed', dualState);
        if (dualState.active) schedulePush(50);
      }
    } catch (e) {
      core.log('dualscreen', 'status failed: ' + e);
    }
    return dualState;
  }

  async function saveConfig(patch) {
    const next = await core.call('terra_dualscreen_configure', {
      ...patch,
      inRide: isRideLike(),
    });
    if (next && typeof next === 'object') {
      dualState = { ...dualState, ...next };
      applyDualBodyClasses();
      core.bus.emit('terra:dualscreen-changed', dualState);
      if (dualState.active) pushNow(true);
    }
    return dualState;
  }

  // ── Element tagging for interactive Mirror / Popup clicks ──
  const mirIdMap = new WeakMap();
  const idToEl = new Map();
  let mirSeq = 1;

  function ensureMirId(el) {
    let id = mirIdMap.get(el);
    if (!id) {
      id = 'm' + (mirSeq++);
      mirIdMap.set(el, id);
    }
    idToEl.set(id, el);
    if (el.getAttribute('data-terra-mir-id') !== id) {
      el.setAttribute('data-terra-mir-id', id);
    }
    return id;
  }

  const INTERACTIVE_SEL = [
    'button', 'a', 'input', 'select', 'textarea', 'summary', 'label',
    '[role="button"]', '.action-button', '.lifecycle-button', '.hud-pill',
    '.list-pill', '.presence-toggle', '.map-chooser', '.compass', '.menu-backing',
    '.menu-tab', '.menu-option', '.range-btn', '.device-item', '.device-row',
    '.hotkey-btn', '.hotkey-binding',
    '.terra-switch', '.terra-link', '.terra-btn', '.terra-btn-primary',
    '.terra-btn-danger', '.terra-chip', '.terra-x', '[onclick]',
  ].join(',');

  function tagInteractive(root) {
    if (!root) return;
    if (root.matches && root.matches(INTERACTIVE_SEL)) ensureMirId(root);
    for (const el of root.querySelectorAll(INTERACTIVE_SEL)) {
      ensureMirId(el);
    }
  }

  let canvasSeq = 1;
  const canvasMap = new WeakMap();
  function ensureCanvasId(cvs) {
    let id = canvasMap.get(cvs);
    if (!id) {
      id = 'c' + (canvasSeq++);
      canvasMap.set(cvs, id);
    }
    if (cvs.getAttribute('data-terra-canvas-id') !== id) {
      cvs.setAttribute('data-terra-canvas-id', id);
    }
    return id;
  }

  function cloneCleanHtml(root) {
    if (!root) return '';
    tagInteractive(root);
    for (const cvs of root.querySelectorAll('canvas')) {
      ensureCanvasId(cvs);
    }
    const clone = root.cloneNode(true);
    for (const s of clone.querySelectorAll('script')) s.remove();

    // Sync live form state into cloned attributes so innerHTML captures them
    const origInputs = root.querySelectorAll('input, select, textarea');
    const cloneInputs = clone.querySelectorAll('input, select, textarea');
    const len = Math.min(origInputs.length, cloneInputs.length);
    for (let i = 0; i < len; i++) {
      const o = origInputs[i];
      const c = cloneInputs[i];
      if (o.tagName === 'INPUT') {
        if (o.type === 'checkbox' || o.type === 'radio') {
          if (o.checked) c.setAttribute('checked', 'checked');
          else c.removeAttribute('checked');
        } else {
          c.setAttribute('value', o.value);
        }
      } else if (o.tagName === 'TEXTAREA') {
        c.textContent = o.value;
      } else if (o.tagName === 'SELECT') {
        const opts = c.querySelectorAll('option');
        opts.forEach((opt, idx) => {
          if (idx === o.selectedIndex) opt.setAttribute('selected', 'selected');
          else opt.removeAttribute('selected');
        });
      }
    }
    return clone.outerHTML;
  }

  function quickHash(str) {
    let h = 2166136261;
    for (let i = 0; i < str.length; i++) {
      h ^= str.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return (h >>> 0).toString(36) + ':' + str.length;
  }

  function collectStylesPayload() {
    const links = [];
    for (const l of document.querySelectorAll('link[rel="stylesheet"]')) {
      if (l.href) links.push(l.href);
    }
    const inline = [];
    for (const st of document.querySelectorAll('style')) {
      if (st.getAttribute('data-terra-dual-css') === '1') continue;
      if (st.textContent) inline.push(st.textContent);
    }
    try {
      for (const sheet of document.adoptedStyleSheets || []) {
        let css = '';
        for (const r of sheet.cssRules || []) css += r.cssText + '\n';
        if (css && !css.includes('.terra-dualscreen-main')) inline.push(css);
      }
    } catch {}
    return { links, inline };
  }

  function snapCanvas(cvs, quality = 0.78) {
    try {
      if (!cvs || cvs.width === 0 || cvs.height === 0) return '';
      return cvs.toDataURL('image/webp', quality);
    } catch {
      return '';
    }
  }

  function isElementActuallyShown(el) {
    if (!el || !el.isConnected) return false;
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') return false;
    const r = el.getBoundingClientRect();
    return r.width > 4 && r.height > 4;
  }

  function collectCockpit() {
    const now = performance.now();
    const timer = document.querySelector('.zone-time .timer-elapsed')?.textContent?.trim() || '00:00';
    const lifeBtn = document.querySelector('.zone-time .lifecycle-button');
    const lifecycleTitle = lifeBtn?.getAttribute('title') || '';
    const lifecycleLabel = lifecycleTitle || (lifeBtn?.classList.contains('ico-pause') ? 'Pause' : 'Start');
    const isPaused = lifecycleLabel !== 'Pause';
    const presence = document.querySelector('.zone-people .presence-label')?.textContent?.trim() || '';
    const ping = document.querySelector('.zone-people .riders-ping')?.textContent?.trim() || '';
    const statusPill = [presence, ping].filter(Boolean).join(' · ') || (isRideLike() ? 'Ride' : '');

    let power = '--', powerSub = '0 avg', hr = '--', hrSub = '0 avg',
      cadence = '0', cadenceSub = '0 avg', speed = '0.0', speedUnit = 'kph', speedSub = '0.0 avg',
      dist = '0.00 km', distSub = '', climb = '0 m', climbSub = '', kcal = '0', kcalSub = '0 /hr';

    for (const cell of document.querySelectorAll('.hud .stats-grid .stat-cell')) {
      const val = cell.querySelector('.stat-value')?.textContent?.trim() || '';
      const unit = (cell.querySelector('.stat-unit')?.textContent?.trim() || '').toLowerCase();
      const comp = cell.querySelector('.stat-companion')?.textContent?.replace(/\s+/g, ' ').trim() || '';
      if (unit === 'w') {
        power = val || '--';
        powerSub = comp || '0 avg';
      } else if (unit === 'bpm') {
        hr = val || '--';
        hrSub = comp || '0 avg';
      } else if (unit === 'rpm' || unit === 'spm') {
        cadence = val || '0';
        cadenceSub = comp || '0 avg';
      } else if (unit === 'kph' || unit === 'mph' || unit === 'km/h') {
        speed = val || '0.0';
        speedUnit = unit;
        speedSub = comp || '0.0 avg';
      } else if (unit === 'km' || unit === 'mi') {
        dist = `${val || '0.00'} ${unit}`;
        distSub = comp;
      } else if (unit === 'climb' || unit === 'm' || unit === 'ft') {
        climb = `${val || '0'} m`;
        climbSub = comp;
      } else if (unit === 'kcal') {
        kcal = val || '0';
        kcalSub = comp || '0 /hr';
      }
    }

    const rideFrame = core.ride && core.ride.latest;
    const ego = rideFrame && rideFrame.ego;
    let wkg = '';
    if (ego && Number.isFinite(+ego.wkg)) {
      wkg = (+ego.wkg).toFixed(1);
    } else if (Number.isFinite(parseFloat(power)) && window.btApp?.riderController?.ego) {
      const massG = window.btApp.riderController.ego.config?.mass || 75000;
      wkg = ((parseFloat(power) * 1000) / massG).toFixed(1);
    }

    const grade = document.querySelector('.zone-road .gauge.grade .gauge-value')?.textContent?.trim() || '0.0';
    const draft = document.querySelector('.zone-road .gauge.draft .gauge-value')?.textContent?.trim() || '0';

    // Realistic gears readout if active
    let gearText = '';
    let ergMode = false;
    const gearHud = document.getElementById('terra-gear-hud');
    if (gearHud && isElementActuallyShown(gearHud)) {
      const txt = gearHud.textContent.replace(/\s+/g, ' ').trim();
      if (txt) {
        gearText = txt.slice(0, 28);
        ergMode = /\berg\b/i.test(txt);
      }
    }

    // Minimap snapshot (~3 Hz)
    const mapCvs = document.querySelector('.zone-place .map-container canvas');
    if (mapCvs && now - lastMapSnapAt >= 320) {
      lastMapSnapAt = now;
      const snap = snapCanvas(mapCvs, 0.75);
      if (snap) lastMapSnapUrl = snap;
    }
    const mapZoomLabel = document.querySelector('.zone-place .map-chooser')?.textContent?.trim() || '';

    // Elevation profile SVG + cursor
    const elevSvgEl = document.querySelector('.panel-profile .elev-graph svg');
    const elevSvg = elevSvgEl ? elevSvgEl.outerHTML : '';
    const elevSvgHash = elevSvg ? quickHash(elevSvg) : '';
    let elevPct = 0;
    const cursorEl = document.querySelector('.panel-profile .elev-cursor');
    if (cursorEl) {
      const m = (cursorEl.getAttribute('style') || '').match(/([\d.]+)%/);
      if (m) elevPct = parseFloat(m[1]);
    }

    // Terra ClimbPro summary if present
    let climbBanner = null;
    const cpRoot = document.querySelector('.tr-cp-root');
    if (cpRoot && isElementActuallyShown(cpRoot)) {
      const title = cpRoot.querySelector('.tr-cp-title')?.textContent?.trim() || '';
      const distEl = cpRoot.querySelector('.tr-cp-dist, .tr-cp-rem')?.textContent?.trim() || '';
      const gradeEl = cpRoot.querySelector('.tr-cp-grade')?.textContent?.trim() || '';
      if (title && !/waiting/i.test(title)) {
        climbBanner = { title, meta: [distEl, gradeEl].filter(Boolean).join(' · ') };
      }
    }

    // Workout banner if active
    let workoutBanner = null;
    const woPanel = document.querySelector('.zone-effort .panel-workout');
    if (woPanel && !woPanel.classList.contains('hidden') && isElementActuallyShown(woPanel)) {
      const txt = woPanel.textContent.replace(/\s+/g, ' ').trim();
      if (txt) workoutBanner = { title: 'Workout', meta: txt.slice(0, 48) };
    }

    // Nearby riders (up to 6) centered around .rider.is-me
    const allRiderEls = [...document.querySelectorAll('.zone-people .riders-main .rider')];
    const hasGroups = allRiderEls.some((r) => r.classList.contains('is-group'));
    let meIdx = allRiderEls.findIndex((r) => r.classList.contains('is-me') && !r.classList.contains('is-group'));
    if (meIdx < 0) meIdx = allRiderEls.findIndex((r) => r.classList.contains('is-me'));
    const maxRiders = 6;
    let startIdx = 0;
    if (allRiderEls.length > maxRiders && meIdx >= 0) {
      startIdx = Math.max(0, Math.min(allRiderEls.length - maxRiders, meIdx - Math.floor(maxRiders / 2)));
    }
    const riders = allRiderEls.slice(startIdx, startIdx + maxRiders).map((rEl, offset) => ({
      pos: hasGroups ? null : startIdx + offset + 1,
      isMe: rEl.classList.contains('is-me'),
      gap: rEl.querySelector('.rider-distance')?.textContent?.replace(/\s+/g, ' ').trim() || '--',
      name: rEl.querySelector('.rider-name')?.textContent?.replace(/\s+/g, ' ').trim() || '',
      stat: rEl.querySelector('.rider-stat-value')?.textContent?.replace(/\s+/g, ' ').trim() || '',
    }));

    // Check if Biketerra's ride-menu or chat panel is open in Cockpit mode
    let popupHtml = '';
    let popupTitle = '';
    let popupHash = '';
    const menuEl = document.querySelector('.ride-menu');
    const chatEl = document.querySelector('.zone-people .panel-chat');
    if (menuEl && isElementActuallyShown(menuEl) && menuEl.innerHTML.trim().length > 0) {
      popupTitle = 'Ride Menu';
      popupHtml = cloneCleanHtml(menuEl);
      popupHash = quickHash(popupHtml);
    } else if (chatEl && !chatEl.classList.contains('hidden') && isElementActuallyShown(chatEl)) {
      popupTitle = 'Ride Chat';
      popupHtml = cloneCleanHtml(chatEl);
      popupHash = quickHash(popupHtml);
    }

    return {
      timer,
      statusPill,
      isPaused,
      lifecycleLabel,
      lifecycleTitle,
      power,
      powerSub,
      wkg,
      hr,
      hrSub,
      cadence,
      cadenceSub,
      speed,
      speedUnit,
      speedSub,
      dist,
      distSub,
      climb,
      climbSub,
      grade,
      draft,
      kcal,
      kcalSub,
      gearText,
      ergMode,
      mapImg: lastMapSnapUrl,
      mapZoomLabel,
      elevSvg,
      elevSvgHash,
      elevPct,
      climbBanner,
      workoutBanner,
      riders,
      popupHtml,
      popupTitle,
      popupHash,
    };
  }

  function collectModal() {
    const modalEl = document.querySelector('.terra-overlay');
    const modalHtml = modalEl ? cloneCleanHtml(modalEl) : '';
    const modalHash = quickHash(modalHtml);
    return { modalHtml, modalHash };
  }

  function collectMirror() {
    const now = performance.now();
    const timer = document.querySelector('.zone-time .timer-elapsed')?.textContent?.trim() || '00:00';
    const presence = document.querySelector('.zone-people .presence-label')?.textContent?.trim() || '';
    const ping = document.querySelector('.zone-people .riders-ping')?.textContent?.trim() || '';
    const statusPill = [presence, ping].filter(Boolean).join(' · ') || 'Mirror';

    const hudEl = document.querySelector('.hud');
    const hudHtml = hudEl ? cloneCleanHtml(hudEl) : '';
    const hudHash = quickHash(hudHtml);

    const addonSelectors = [
      '#terra-fps-stats',
      '#fps-monitor-hud',
      '#terra-gear-hud',
      '#tvc-hud',
      '#btz-console-hud',
      '#btz-ring',
      '#terra-export-mode-panel',
    ];
    const cpRoot = document.querySelector('.tr-cp-root');
    const addonParts = [];
    if (cpRoot && (!hudEl || !hudEl.contains(cpRoot)) && isElementActuallyShown(cpRoot)) {
      addonParts.push(cloneCleanHtml(cpRoot));
    }
    for (const sel of addonSelectors) {
      const el = document.querySelector(sel);
      if (el && isElementActuallyShown(el)) addonParts.push(cloneCleanHtml(el));
    }
    const addonsHtml = addonParts.join('');
    const addonsHash = quickHash(addonsHtml);

    const menuParts = [];
    for (const sel of ['.menu-backing', '.ride-menu', '.load-overlay', '.cinecam-warn', '.spectating-name']) {
      const el = document.querySelector(sel);
      if (el && isElementActuallyShown(el) && (sel === '.menu-backing' || el.innerHTML.trim().length > 0)) {
        menuParts.push(cloneCleanHtml(el));
      }
    }
    const menuHtml = menuParts.join('');
    const menuHash = quickHash(menuHtml);

    // Canvas snapshots inside .hud / addons
    const canvases = {};
    const mapCvs = document.querySelector('.zone-place .map-container canvas');
    if (mapCvs) {
      const cid = ensureCanvasId(mapCvs);
      if (!lastMapSnapUrl || now - lastMapSnapAt >= 320) {
        lastMapSnapAt = now;
        const url = snapCanvas(mapCvs, 0.75);
        if (url) lastMapSnapUrl = url;
      }
      if (lastMapSnapUrl) canvases[cid] = lastMapSnapUrl;
    }

    const graphCanvases = [...document.querySelectorAll('.hud canvas.terra-dark-graph')];
    const missingGraphSnap = graphCanvases.some((cvs) => !graphSnapCache.has(ensureCanvasId(cvs)));
    if (missingGraphSnap || now - lastGraphSnapAt >= 240) {
      lastGraphSnapAt = now;
      for (const cvs of graphCanvases) {
        const cid = ensureCanvasId(cvs);
        const url = snapCanvas(cvs, 0.78);
        if (url) graphSnapCache.set(cid, url);
      }
    }
    for (const [cid, url] of graphSnapCache) {
      canvases[cid] = url;
    }

    return {
      timer,
      statusPill,
      hudHtml,
      hudHash,
      addonsHtml,
      addonsHash,
      menuHtml,
      menuHash,
      canvases,
    };
  }

  let lastStylesHash = '';
  let lastPayloadSig = '';

  async function pushNow(force = false) {
    if (!dualState.active || !isRideLike()) return;
    if (pushBusy) {
      schedulePush(80);
      return;
    }
    const now = performance.now();
    if (!force && now - lastPushAt < 140) {
      schedulePush(140 - (now - lastPushAt));
      return;
    }
    pushBusy = true;
    lastPushAt = now;
    try {
      const payloadObj = { mode: dualState.mode };
      const styles = collectStylesPayload();
      const sHash = quickHash(JSON.stringify(styles));
      if (force || sHash !== lastStylesHash) {
        lastStylesHash = sHash;
        payloadObj.styles = styles;
      }
      payloadObj.modal = collectModal();
      if (dualState.mode === 'mirror') {
        payloadObj.mirror = collectMirror();
      } else {
        payloadObj.cockpit = collectCockpit();
      }
      const raw = JSON.stringify(payloadObj);
      const sig = quickHash(raw);
      if (force || sig !== lastPayloadSig) {
        lastPayloadSig = sig;
        await core.call('terra_dualscreen_push', { payload: raw });
      }
    } catch {
      /* ignore transient IPC errors during unload */
    } finally {
      pushBusy = false;
    }
  }

  function clickBtn(el) {
    if (!el) return;
    try { el.focus && el.focus({ preventScroll: true }); } catch {}
    const tag = el.tagName;
    if (
      tag === 'LABEL' ||
      tag === 'SUMMARY' ||
      (tag === 'A' && el.hasAttribute('href')) ||
      (tag === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio'))
    ) {
      try { el.click(); return; } catch {}
    }
    const opts = { bubbles: true, cancelable: true, view: window, detail: 1, button: 0 };
    try {
      el.dispatchEvent(new PointerEvent('pointerdown', { ...opts, pointerType: 'touch', isPrimary: true }));
      el.dispatchEvent(new MouseEvent('mousedown', opts));
      el.dispatchEvent(new PointerEvent('pointerup', { ...opts, pointerType: 'touch', isPrimary: true }));
      el.dispatchEvent(new MouseEvent('mouseup', opts));
      el.dispatchEvent(new MouseEvent('click', opts));
    } catch {
      try { el.click(); } catch {}
    }
  }

  function findActionBtnByImgAlt(alt) {
    for (const btn of document.querySelectorAll('.action-button')) {
      const img = btn.querySelector('img');
      if (img && (img.getAttribute('alt') || '').toLowerCase() === alt.toLowerCase()) return btn;
    }
    return null;
  }

  window.__terraDualAction = (act) => {
    if (!act || typeof act !== 'object') return;
    if (act.type === 'mirMetrics') {
      const w = Number(act.graphW);
      const h = Number(act.graphH);
      if (w > 40 && h > 20) {
        const rootStyle = document.documentElement.style;
        const nextW = `${Math.floor(w)}px`;
        const nextFont = `${((Math.floor(h) + 0.35) / 3.2).toFixed(3)}px`;
        const changed =
          rootStyle.getPropertyValue('--terra-mir-graph-w') !== nextW ||
          rootStyle.getPropertyValue('--terra-mir-graph-font') !== nextFont;
        rootStyle.setProperty('--terra-mir-graph-w', nextW);
        rootStyle.setProperty('--terra-mir-graph-font', nextFont);
        if (changed) {
          graphSnapCache.clear();
          lastGraphSnapAt = 0;
          setTimeout(() => pushNow(true), 260);
          setTimeout(() => pushNow(true), 600);
        }
      }
      return;
    }
    if (act.type === 'openSettings') {
      core.bus.emit('terra:open-addons', {});
      setTimeout(() => pushNow(true), 60);
      return;
    }
    if (act.type === 'mirClick' && act.mirId) {
      const el = idToEl.get(act.mirId) || document.querySelector(`[data-terra-mir-id="${CSS.escape(act.mirId)}"]`);
      clickBtn(el);
      setTimeout(() => pushNow(true), 50);
      return;
    }
    if (act.type === 'mirLiveInput' && act.mirId) {
      const el = idToEl.get(act.mirId) || document.querySelector(`[data-terra-mir-id="${CSS.escape(act.mirId)}"]`);
      if (!el) return;
      el.value = act.value != null ? String(act.value) : '';
      el.dispatchEvent(new Event('input', { bubbles: true }));
      setTimeout(() => pushNow(true), 50);
      return;
    }
    if (act.type === 'mirInput' && act.mirId) {
      const el = idToEl.get(act.mirId) || document.querySelector(`[data-terra-mir-id="${CSS.escape(act.mirId)}"]`);
      if (!el) return;
      if (el.type === 'checkbox' || el.type === 'radio') {
        el.checked = !!act.checked;
      } else {
        el.value = act.value != null ? String(act.value) : '';
      }
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
      setTimeout(() => pushNow(true), 50);
      return;
    }
    if (act.type === 'mirEnter' && act.mirId) {
      const el = idToEl.get(act.mirId) || document.querySelector(`[data-terra-mir-id="${CSS.escape(act.mirId)}"]`);
      if (!el) return;
      el.value = act.value != null ? String(act.value) : '';
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true, cancelable: true }));
      el.dispatchEvent(new KeyboardEvent('keyup', { key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true, cancelable: true }));
      setTimeout(() => pushNow(true), 60);
      return;
    }
    if (act.type === 'cockpitAction') {
      switch (act.name) {
        case 'lifecycle':
          clickBtn(document.querySelector('.zone-time .lifecycle-button'));
          break;
        case 'menu':
          clickBtn(findActionBtnByImgAlt('Menu'));
          break;
        case 'camera':
          clickBtn(findActionBtnByImgAlt('Camera'));
          break;
        case 'screenshot':
          clickBtn(document.querySelector('.action-button[data-screenshot]') || findActionBtnByImgAlt('Screenshot'));
          break;
        case 'chat':
          clickBtn(findActionBtnByImgAlt('Chat'));
          break;
        case 'mapZoom':
          clickBtn(document.querySelector('.zone-place .map-chooser'));
          break;
        case 'mapCompass':
          clickBtn(document.querySelector('.zone-place .compass'));
          break;
        case 'closePopup': {
          const menuEl = document.querySelector('.ride-menu');
          if (menuEl && isElementActuallyShown(menuEl)) {
            clickBtn(document.querySelector('.menu-backing') || findActionBtnByImgAlt('Menu'));
          } else {
            clickBtn(findActionBtnByImgAlt('Chat'));
          }
          break;
        }
      }
      setTimeout(() => pushNow(true), 60);
    }
  };

  // SPA navigation tracking
  const syncNav = () => {
    const nextInRide = isRideLike();
    if (nextInRide !== dualState.inRide) {
      dualState.inRide = nextInRide;
      if (dualState.supported) {
        core.call('terra_dualscreen_configure', { inRide: nextInRide }).catch(() => {});
      }
    }
    applyDualBodyClasses();
    if (dualState.active) schedulePush(80);
  };
  for (const m of ['pushState', 'replaceState']) {
    const orig = history[m];
    history[m] = function (...a) {
      const r = orig.apply(this, a);
      setTimeout(syncNav, 0);
      return r;
    };
  }
  addEventListener('popstate', syncNav);
  addEventListener('hashchange', syncNav);

  // Periodic push while dual-screen ride is active (~5 Hz)
  setInterval(() => {
    if (dualState.active && isRideLike()) pushNow(false);
  }, 200);

  window.__terraDualScreen = {
    getState: () => dualState,
    syncStatus,
    saveConfig,
    pushNow,
  };

  ensureCss();
  syncStatus();
})();
