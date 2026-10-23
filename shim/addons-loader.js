// Terra addon loader. Runs in the page after bluetooth-shim.js.
// Rust evaluates addon code and calls window.__terraAddons.start(meta, fn).
(() => {
  if (window.top !== window || window.__terraAddons) return;
  const core = window.__terraCore;
  if (!core) return;
  const { bus } = core;

  const active = new Map();  // id -> { cleanups: [], nav: Set }
  const pending = new Map(); // id -> timer (evaluated by Rust, waiting for start())

  const addSheet = (css) => {
    try {
      const s = new CSSStyleSheet();
      s.replaceSync(css);
      document.adoptedStyleSheets = [...document.adoptedStyleSheets, s];
      return () => { document.adoptedStyleSheets = document.adoptedStyleSheets.filter((x) => x !== s); };
    } catch {
      const el = document.createElement('style');
      el.textContent = css;
      (document.head || document.documentElement).append(el);
      return () => el.remove();
    }
  };

  const keyCode = (k) => {
    if (/^[a-z]$/i.test(k)) return 'Key' + k.toUpperCase();
    if (/^[0-9]$/.test(k)) return 'Digit' + k;
    return k === ' ' ? 'Space' : k;
  };

  const fail = (id, e) => core.log('addon', `${id} error: ${e && e.stack ? e.stack : e}`);

  function makeTerra(meta, state) {
    const cleanup = (fn) => { state.cleanups.push(fn); return fn; };
    const on = (topic, cb) => cleanup(bus.on(topic, cb));
    const keyEvent = (type, key, opts = {}) => {
      const target = opts.target || document.activeElement || document.body;
      target.dispatchEvent(new KeyboardEvent(type, {
        key, code: keyCode(key), bubbles: true, cancelable: true, composed: true, ...opts.init,
      }));
    };
    const api = {
      id: meta.id, name: meta.name, version: meta.version,
      log: (...a) => core.log('addon:' + meta.id, a.map((x) => (typeof x === 'string' ? x : JSON.stringify(x))).join(' ')),
      onCleanup: cleanup,
      onNavigate: (fn) => { state.nav.add(fn); },
      waitFor: (selector, { timeout = 10000, root = document } = {}) => new Promise((res, rej) => {
        const find = () => root.querySelector(selector);
        const hit = find();
        if (hit) return res(hit);
        const done = () => { mo.disconnect(); clearTimeout(t); };
        const mo = new MutationObserver(() => { const e = find(); if (e) { done(); res(e); } });
        mo.observe(root.documentElement || root, { childList: true, subtree: true });
        const t = setTimeout(() => { done(); rej(new Error('waitFor timeout: ' + selector)); }, timeout);
        cleanup(done);
      }),
      storage: {
        get: async (key, dflt) => {
          const v = await core.call('addon_storage_get', { id: meta.id, key });
          return v === null || v === undefined ? dflt : v;
        },
        set: (key, value) => core.call('addon_storage_set', { id: meta.id, key, value }),
        remove: (key) => core.call('addon_storage_remove', { id: meta.id, key }),
      },
      // Settings declared in the manifest and edited in the cog menu. Password settings are write-only:
      // the page can't read them back (use {{setting:KEY}} placeholders in terra.http requests instead).
      settings: {
        all: async () => (await core.call('addon_settings_get', { id: meta.id })).values,
        get: async (key) => (await core.call('addon_settings_get', { id: meta.id })).values[key],
        has: async (key) => {
          const r = await core.call('addon_settings_get', { id: meta.id });
          return r.secretsSet.includes(key) || (r.values[key] !== undefined && r.values[key] !== null);
        },
        set: (key, value) => core.call('addon_settings_set', { id: meta.id, key, value }),
        onChange: (cb) => on('settings:' + meta.id, cb), // fires when a value is saved in the cog menu
        open: () => bus.emit('terra:open-addons', { expand: meta.id }), // open the cog menu on this addon's settings
      },
      // Requests made by Terra itself (no CORS). https only, to hosts listed in the manifest's "connect".
      // req: { method, url, headers, body, basic: {user, password}, timeoutMs,
      //        form: { fields, file: {field, filename, contentType, base64} } } -> { status, body }
      http: { request: (req) => core.call('addon_http', { addon: meta.id, req }) },
      // Saves bytes (Uint8Array) to ~/Downloads/Terra; resolves to the saved path.
      files: { save: (filename, bytes) => core.call('addon_save_file', { addon: meta.id, filename, base64: core.toB64(bytes) }) },
      // Biketerra's ride broadcast, relayed to the Rust backend (which keeps a ~5 min history).
      // Frame: { ego: {speed, power, cadence, heartRate, ...}, riders: [...], timestamp }, plus
      // whatever else the page broadcasts. help-center/live-broadcast-data.
      ride: {
        // Most recent frame (in-page, immediate).
        latest: () => (core.ride && core.ride.latest) || null,
        // Live updates, ~2Hz. Returns a cleanup fn.
        onRide: (cb) => on('biketerra:ride', cb),
        // Up to ~5 min of frames, oldest to newest; `limit` caps the count.
        history: async (limit) => (await core.call('ride_history', { limit: limit == null ? null : limit })) ?? [],
      },
      bus: { on, emit: (topic, data) => bus.emit(topic, data) },
      ble: {
        devices: () => [...core.devices.values()].map((d) => ({ id: d.id, name: d.name, connected: d.gatt.connected })),
        // Pair a device for this addon (always shows the chooser); Terra remembers it for this addon only.
        request: (options) => navigator.bluetooth.requestDevice({ ...options, __terraOwner: meta.id }),
        // Devices this addon paired earlier, as BluetoothDevice objects (no UI).
        remembered: async () => (await core.call('ble_remembered', { owner: meta.id })).map((i) => core.deviceFor(i)),
        // Let the page (not only this addon) use the devices this addon paired. Resets on every page load.
        setShared: (on) => core.call('ble_release', { owner: meta.id, released: !!on }),
        onNotify: (cb) => on('ble:notify', cb),       // { deviceId, name, characteristic, value: DataView }
        onPaired: (cb) => on('ble:paired', cb),
        onConnect: (cb) => on('ble:connected', cb),
        onDisconnect: (cb) => on('ble:disconnected', cb),
      },
      keys: {
        down: (key, opts) => keyEvent('keydown', key, opts),
        up: (key, opts) => keyEvent('keyup', key, opts),
        tap: async (key, ms = 40, opts) => {
          keyEvent('keydown', key, opts);
          await new Promise((r) => setTimeout(r, ms));
          keyEvent('keyup', key, opts);
        },
      },
      ui: { addStyle: (css) => cleanup(addSheet(css)) },
    };
    return Object.freeze(api);
  }

  function start(meta, fn) {
    clearTimeout(pending.get(meta.id));
    pending.delete(meta.id);
    if (active.has(meta.id)) return;
    const state = { cleanups: [], nav: new Set() };
    active.set(meta.id, state);
    for (const css of meta.css || []) state.cleanups.push(addSheet(css));
    core.log('addon', `started ${meta.id}`);
    try {
      const r = fn(makeTerra(meta, state));
      if (r && r.catch) r.catch((e) => fail(meta.id, e));
    } catch (e) { fail(meta.id, e); }
  }

  function stop(id) {
    const s = active.get(id);
    if (!s) return;
    active.delete(id);
    for (const f of s.cleanups.reverse()) { try { f(); } catch {} }
    core.log('addon', `stopped ${id}`);
  }

  let busy = false, again = false, timer = null;
  const schedule = (ms = 100) => { clearTimeout(timer); timer = setTimeout(activate, ms); };

  async function activate() {
    if (busy) { again = true; return; }
    busy = true;
    try {
      const res = await core.call('addons_activate', {
        url: location.href,
        active: [...active.keys(), ...pending.keys()],
      });
      res.stop.forEach(stop);
      for (const id of res.started) {
        pending.set(id, setTimeout(() => {
          pending.delete(id);
          if (!active.has(id)) core.log('addon', `${id} did not start (syntax error in its JS?)`);
        }, 3000));
      }
    } catch (e) { core.log('addon', 'activate failed: ' + e); }
    finally { busy = false; if (again) { again = false; schedule(); } }
  }

  const navigated = () => {
    for (const s of active.values())
      for (const cb of s.nav) { try { cb(location.href); } catch (e) { console.error(e); } }
    schedule();
  };
  for (const m of ['pushState', 'replaceState']) {
    const orig = history[m];
    history[m] = function (...a) { const r = orig.apply(this, a); setTimeout(navigated, 0); return r; };
  }
  addEventListener('popstate', navigated);
  addEventListener('hashchange', navigated);

  // Reload with F5 / Ctrl+R: addon folders are rescanned on every page load.
  addEventListener('keydown', (e) => {
    if (e.key === 'F5' || (e.ctrlKey && e.key.toLowerCase() === 'r')) { e.preventDefault(); location.reload(); }
  }, true);

  window.__terraAddons = { start, stop, active, refresh: () => schedule(0) };

  if (document.readyState === 'loading') addEventListener('DOMContentLoaded', () => schedule(0));
  else schedule(0);
})();
