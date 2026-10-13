// Web Bluetooth shim backed by Terra's Rust BLE broker (btleplug).
(() => {
  if (window.__terraBtShim) return;
  window.__terraBtShim = true;

  const call = (cmd, args) => {
    const T = window.__TAURI_INTERNALS__;
    return T ? T.invoke(cmd, args) : Promise.reject('Terra IPC unavailable');
  };
  const log = (kind, detail) => {
    console.log('[terra-bt]', kind, detail);
    try { call('shim_log', { kind, detail: detail ?? null }).catch(() => {}); } catch {}
  };

  // What the page asks the Bluetooth API to do (shows up in the terminal as `[shim] gatt.*`).
  const hexOf = (v) => [...new Uint8Array(v.buffer ?? v, v.byteOffset ?? 0, v.byteLength)]
    .map((b) => b.toString(16).padStart(2, '0')).join(' ');
  const trace = (kind, detail) => log('gatt.' + kind, detail);

  // ---- UUID helpers ----
  const base = (n) => n.toString(16).padStart(8, '0') + '-0000-1000-8000-00805f9b34fb';
  const ALIASES = {
    generic_access: 0x1800, device_information: 0x180a, battery_service: 0x180f,
    heart_rate: 0x180d, cycling_power: 0x1818, cycling_speed_and_cadence: 0x1816,
    fitness_machine: 0x1826,
    battery_level: 0x2a19, heart_rate_measurement: 0x2a37, body_sensor_location: 0x2a38,
    csc_measurement: 0x2a5b, sensor_location: 0x2a5d, cycling_power_measurement: 0x2a63,
    cycling_power_feature: 0x2a65, cycling_power_control_point: 0x2a66,
    fitness_machine_feature: 0x2acc, indoor_bike_data: 0x2ad2, training_status: 0x2ad3,
    supported_resistance_level_range: 0x2ad6, supported_power_range: 0x2ad8,
    fitness_machine_control_point: 0x2ad9, fitness_machine_status: 0x2ada,
  };
  const uuid = (u) => {
    if (typeof u === 'number') return base(u);
    const s = String(u).toLowerCase();
    if (s in ALIASES) return base(ALIASES[s]);
    if (/^[0-9a-f]{4}$/.test(s) || /^[0-9a-f]{8}$/.test(s)) return base(parseInt(s, 16));
    return s;
  };

  // ---- byte helpers ----
  const toDV = (b64) => {
    const bin = atob(b64), a = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) a[i] = bin.charCodeAt(i);
    return new DataView(a.buffer);
  };
  const toB64 = (v) => {
    const a = v instanceof ArrayBuffer ? new Uint8Array(v)
      : new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
    let s = '';
    for (let i = 0; i < a.length; i += 0x8000) s += String.fromCharCode(...a.subarray(i, i + 0x8000));
    return btoa(s);
  };
  const notFound = (m) => new DOMException(String(m), 'NotFoundError');

  const devices = new Map();
  let silentNext = false; // set by auto-pair so Rust never opens the chooser
  const autoPaired = new Set(); // device ids auto-pair has already handed to the page

  // Tiny in-page event bus shared with the addon loader.
  const bus = (() => {
    const m = new Map();
    return {
      on(t, cb) {
        if (!m.has(t)) m.set(t, new Set());
        m.get(t).add(cb);
        return () => m.get(t)?.delete(cb);
      },
      emit(t, d) {
        for (const cb of [...(m.get(t) || [])]) {
          try { cb(d); } catch (e) { console.error('[terra-bus]', t, e); }
        }
      },
    };
  })();

  // ---- ordering that a browser guarantees and our IPC does not ----
  // 1) GATT operations on one device run strictly one at a time, in call order.
  // 2) A notification that arrives while a write is in flight is held until the write's promise has been
  //    delivered to the page, so `await write(); then expect the indication` can't see them reversed.
  const opTail = new Map(); // device id -> promise of the last queued operation
  const writing = new Map(); // device id -> writes currently in flight
  const held = new Map(); // device id -> { q: [deliver fns], t: forced-flush timer }
  const flushHeld = (id, force) => {
    const h = held.get(id);
    if (!h || (!force && (writing.get(id) || 0) > 0)) return;
    held.delete(id);
    clearTimeout(h.t);
    for (const f of h.q) { try { f(); } catch (e) { console.error('[terra-bt]', e); } }
  };
  const gattOp = (id, fn, isWrite = false, label = '') => {
    const run = (opTail.get(id) || Promise.resolve()).then(() => {
      if (isWrite) writing.set(id, (writing.get(id) || 0) + 1);
      // watchdog: everything queued behind a hung operation would wait forever
      const t = setTimeout(() => log('GATT operation still pending after 5 s', { id, op: label }), 5000);
      return (async () => fn())()
        .catch((e) => { log('GATT operation failed', { op: label, error: String((e && e.message) || e) }); throw e; })
        .finally(() => clearTimeout(t));
    });
    const settled = run.then(() => {}, () => {});
    opTail.set(id, settled);
    if (isWrite) settled.then(() => {
      writing.set(id, writing.get(id) - 1);
      setTimeout(() => flushHeld(id), 0); // after the caller's continuation has run
    });
    return run;
  };

  class RemoteCharacteristic extends EventTarget {
    constructor(service, info) {
      super();
      this.service = service;
      this.uuid = info.uuid;
      this.value = null;
      this.properties = {
        broadcast: false, read: info.read, writeWithoutResponse: info.writeWithoutResponse,
        write: info.write, notify: info.notify, indicate: info.indicate,
        authenticatedSignedWrites: false, reliableWrite: false, writableAuxiliaries: false,
      };
    }
    get _a() { return { id: this.service.device.id, service: this.service.uuid, characteristic: this.uuid }; }
    async readValue() {
      trace('read', this.uuid);
      this.value = toDV(await gattOp(this._a.id, () => call('ble_read', this._a), false, 'read ' + this.uuid));
      this.dispatchEvent(new Event('characteristicvaluechanged'));
      return this.value;
    }
    async writeValue(v) { return this.writeValueWithResponse(v); }
    async writeValueWithResponse(v) {
      trace('write', { chr: this.uuid, mode: 'req', hex: hexOf(v) });
      const args = { ...this._a, value: toB64(v), withResponse: true }; // copy the bytes now, like a browser
      await gattOp(args.id, () => call('ble_write', args), true, 'write ' + this.uuid);
    }
    async writeValueWithoutResponse(v) {
      trace('write', { chr: this.uuid, mode: 'cmd', hex: hexOf(v) });
      const args = { ...this._a, value: toB64(v), withResponse: false };
      await gattOp(args.id, () => call('ble_write', args), true, 'write ' + this.uuid);
    }
    async startNotifications() { trace('startNotifications', this.uuid); await gattOp(this._a.id, () => call('ble_subscribe', this._a), false, 'subscribe ' + this.uuid); return this; }
    async stopNotifications() { trace('stopNotifications', this.uuid); await gattOp(this._a.id, () => call('ble_unsubscribe', this._a), false, 'unsubscribe ' + this.uuid); return this; }
    async getDescriptor() { throw new DOMException('Descriptors not supported', 'NotSupportedError'); }
    async getDescriptors() { return []; }
  }

  class RemoteService {
    constructor(device, info) {
      this.device = device;
      this.uuid = info.uuid;
      this.isPrimary = true;
      this.chars = info.characteristics.map((c) => new RemoteCharacteristic(this, c));
    }
    async getCharacteristic(u) {
      const id = uuid(u), c = this.chars.find((x) => x.uuid === id);
      if (!c) throw notFound(`characteristic ${id} not found`);
      return c;
    }
    async getCharacteristics(u) {
      const r = u === undefined ? this.chars : this.chars.filter((x) => x.uuid === uuid(u));
      if (!r.length) throw notFound('no matching characteristics');
      return r;
    }
  }

  class RemoteGATTServer {
    constructor(device) { this.device = device; this.connected = false; this.services = null; }
    async connect() {
      // Like a browser: connecting an already-connected server returns it unchanged (same service and
      // characteristic objects, so listeners attached earlier keep working); parallel calls share one attempt.
      if (this.connected && this.services) return this;
      if (this._connecting) return this._connecting;
      trace('connect', { id: this.device.id, name: this.device.name });
      this._connecting = (async () => {
        try {
          const tree = await call('ble_connect', { id: this.device.id }).catch((e) => {
            throw new DOMException(String(e), 'NetworkError');
          });
          this.services = tree.map((s) => new RemoteService(this.device, s));
          this.connected = true;
          trace('connected', { id: this.device.id, services: tree.map((s) => s.uuid) });
          bus.emit('ble:connected', { deviceId: this.device.id, name: this.device.name });
          return this;
        } finally { this._connecting = null; }
      })();
      return this._connecting;
    }
    disconnect() {
      if (!this.connected) return;
      this.connected = false;
      call('ble_disconnect', { id: this.device.id }).catch(() => {});
      bus.emit('ble:disconnected', { deviceId: this.device.id, name: this.device.name });
      this.device.dispatchEvent(new Event('gattserverdisconnected'));
    }
    async getPrimaryService(u) {
      if (!this.connected) throw new DOMException('GATT server not connected', 'NetworkError');
      const id = uuid(u), s = this.services.find((x) => x.uuid === id);
      trace('getPrimaryService', id);
      if (!s) throw notFound(`service ${id} not found`);
      return s;
    }
    async getPrimaryServices(u) {
      if (!this.connected) throw new DOMException('GATT server not connected', 'NetworkError');
      const r = u === undefined ? this.services : this.services.filter((x) => x.uuid === uuid(u));
      if (!r.length) throw notFound('no matching services');
      return r;
    }
  }

  class TerraBluetoothDevice extends EventTarget {
    constructor(info) {
      super();
      this.id = info.id;
      this.name = info.name ?? null;
      this.gatt = new RemoteGATTServer(this);
      this.watchingAdvertisements = false;
    }
    async forget() {
      await call('ble_forget', { id: this.id }).catch(() => {});
      this.gatt.connected = false;
      devices.delete(this.id);
    }
    async watchAdvertisements() { throw new DOMException('Not supported', 'NotSupportedError'); }
  }

  // Hooks called from Rust via webview.eval()
  window.__terra = {
    notify(id, chr, b64) {
      const dv = toDV(b64);
      const deliver = () => {
        const d = devices.get(id);
        bus.emit('ble:notify', { deviceId: id, name: d ? d.name : null, characteristic: chr, value: dv });
        if (!d || !d.gatt.services) return;
        for (const s of d.gatt.services)
          for (const c of s.chars)
            if (c.uuid === chr) {
              c.value = dv;
              c.dispatchEvent(new Event('characteristicvaluechanged'));
            }
      };
      if ((writing.get(id) || 0) > 0 || held.has(id)) { // keep order: hold until the write has been delivered
        let h = held.get(id);
        if (!h) { h = { q: [], t: setTimeout(() => flushHeld(id, true), 500) }; held.set(id, h); }
        h.q.push(deliver);
      } else deliver();
    },
    disconnected(id) {
      const d = devices.get(id);
      if (!d || !d.gatt.connected) return;
      d.gatt.connected = false;
      bus.emit('ble:disconnected', { deviceId: id, name: d.name });
      d.dispatchEvent(new Event('gattserverdisconnected'));
    },
  };

  const bluetooth = {
    async getAvailability() { return true; },
    async getDevices() { return [...devices.values()]; },
    async requestDevice(options = {}) {
      log('requestDevice', options);
      const acceptAll = !!options.acceptAllDevices;
      const conv = (f) => ({
        services: f.services ? f.services.map(uuid) : null,
        name: f.name ?? null,
        namePrefix: f.namePrefix ?? null,
      });
      const filters = (options.filters || []).map(conv);
      const exclusionFilters = (options.exclusionFilters || []).map(conv);
      const owner = options.__terraOwner ?? null; // set by terra.ble.request(): addon-owned pairing
      const silent = silentNext;
      silentNext = false;
      let info;
      try { info = await call('ble_request_device', { filters, exclusionFilters, acceptAll, silent, owner }); }
      catch (e) { window.__terraSettled?.(false); throw notFound(e); }
      if (silent) { // auto-pair: never hand the same device to the page twice
        if (autoPaired.has(info.id)) { window.__terraSettled?.(false); throw notFound('already paired by auto-pair'); }
        autoPaired.add(info.id);
        window.__terraLastPaired = info.id;
      }
      window.__terraSettled?.(true);
      let dev = devices.get(info.id);
      if (!dev) { dev = new TerraBluetoothDevice(info); devices.set(info.id, dev); }
      log('requestDevice -> ', info);
      bus.emit('ble:paired', { deviceId: info.id, name: info.name });
      return dev;
    },
    async requestLEScan() { throw new DOMException('Not supported', 'NotSupportedError'); },
    addEventListener() {}, removeEventListener() {},
  };

  // ---- auto-pair: press Biketerra's own "Pair device..." button for remembered devices ----
  (() => {
    const enabled = () => { try { return localStorage.getItem('terra.autopair') !== '0'; } catch { return true; } };
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    let ranFor = null, running = false, timer = null;

    const findBtn = () => {
      let best = null;
      for (const el of document.querySelectorAll('button, [role="button"], a, div, span')) {
        const t = (el.innerText || '').trim().toLowerCase();
        if (!t.startsWith('pair device') || t.length > 40) continue;
        if (!best || best.contains(el)) best = el; // prefer the innermost match
      }
      return best && (best.closest('button, [role="button"], a') || best);
    };
    const press = (el) => {
      const r = el.getBoundingClientRect();
      const x = r.left + r.width / 2, y = r.top + r.height / 2;
      const target = (r.width && r.height && document.elementFromPoint(x, y)) || el;
      const seq = ['pointerover', 'mouseover', 'pointermove', 'mousemove',
                   'pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click'];
      for (const t of seq) {
        const ptr = t.startsWith('pointer');
        const init = {
          bubbles: true, cancelable: true, composed: true, view: window,
          clientX: x, clientY: y, button: 0, buttons: t.endsWith('down') ? 1 : 0, detail: 1,
          ...(ptr ? { pointerId: 1, pointerType: 'mouse', isPrimary: true } : {}),
        };
        target.dispatchEvent(new (ptr ? PointerEvent : MouseEvent)(t, init));
      }
      return { target, r };
    };

    // resolves once the page has finished connecting to that device (or after `ms`)
    const waitConnected = (id, ms) => new Promise((res) => {
      const d = devices.get(id);
      if (d && d.gatt.connected) return res(true);
      const t = setTimeout(() => { off(); res(false); }, ms);
      const off = bus.on('ble:connected', (e) => { if (e.deviceId === id) { off(); clearTimeout(t); res(true); } });
    });

    async function run() {
      if (running || !enabled()) return;
      running = true;
      try {
        for (let i = 0; i < 6; i++) {
          const b = findBtn();
          if (!b) break;
          silentNext = true;
          const settled = new Promise((res) => {
            window.__terraSettled = res;
            setTimeout(() => res(null), 4000);
          });
          const { target, r } = press(b);
          log('auto-pair press', {
            found: b.outerHTML.slice(0, 300),
            clicked: target === b ? '(same)' : target.outerHTML.slice(0, 300),
            rect: { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) },
          });
          const ok = await settled;
          silentNext = false;
          window.__terraSettled = null;
          log('auto-pair', { attempt: i + 1, paired: ok });
          if (!ok) break;
          await waitConnected(window.__terraLastPaired, 10000); // let the page finish connecting first
          await sleep(500);
        }
      } finally { running = false; silentNext = false; }
    }

    const check = () => {
      timer = null;
      if (!location.pathname.startsWith('/ride') || ranFor === location.href) return;
      if (!findBtn()) return;
      ranFor = location.href;
      setTimeout(run, 800);
    };
    new MutationObserver(() => { if (!timer) timer = setTimeout(check, 500); })
      .observe(document, { childList: true, subtree: true, characterData: true });
  })();

  // In-page Bluetooth device chooser modal for Android & Webview contexts
  let chooserOverlay = null, chooserPollTimer = null;

  window.showChooserModal = () => {
    if (chooserOverlay) return;
    const CSS = `
.terra-chooser-overlay { position: fixed; inset: 0; z-index: 2147483647; display: flex; align-items: center; justify-content: center;
  background: rgba(0,0,0,.65); backdrop-filter: blur(4px); font: 14px system-ui, sans-serif; }
.terra-chooser-modal { width: min(480px, calc(100vw - 32px)); max-height: calc(100vh - 64px); display: flex; flex-direction: column;
  background: #18181f; color: #eee; border-radius: 16px; box-shadow: 0 20px 60px rgba(0,0,0,.7); overflow: hidden; border: 1px solid #2d2d38; }
.terra-chooser-head { display: flex; align-items: center; justify-content: space-between; padding: 16px 20px; border-bottom: 1px solid #2a2a35; }
.terra-chooser-head h3 { margin: 0; font-size: 16px; font-weight: 600; color: #fff; }
.terra-chooser-body { padding: 12px 16px; max-height: 380px; overflow-y: auto; display: flex; flex-direction: column; gap: 8px; }
.terra-chooser-item { display: flex; align-items: center; justify-content: space-between; padding: 12px 14px; background: #22222c; border-radius: 10px; border: 1px solid #2e2e3b; }
.terra-chooser-item:hover { background: #2a2a38; }
.terra-chooser-name { font-weight: 600; color: #fff; font-size: 14px; }
.terra-chooser-sub { font-size: 12px; color: #8a8a9a; margin-top: 2px; }
.terra-chooser-btn { background: #ff6b57; color: #fff; border: 0; border-radius: 8px; padding: 8px 16px; font-weight: 600; cursor: pointer; }
.terra-chooser-btn:hover { background: #ff523b; }
.terra-chooser-foot { display: flex; justify-content: flex-end; padding: 12px 20px; border-top: 1px solid #2a2a35; background: #14141a; }
.terra-chooser-cancel { background: #2a2a35; color: #bbb; border: 0; border-radius: 8px; padding: 8px 16px; cursor: pointer; }
.terra-chooser-cancel:hover { background: #353542; color: #fff; }
`;
    try {
      const s = new CSSStyleSheet(); s.replaceSync(CSS);
      document.adoptedStyleSheets = [...document.adoptedStyleSheets, s];
    } catch {
      const el = document.createElement('style'); el.textContent = CSS;
      document.head.append(el);
    }

    const bodyEl = document.createElement('div');
    bodyEl.className = 'terra-chooser-body';
    bodyEl.innerHTML = '<div style="padding: 24px; text-align: center; color: #8a8a9a;">Scanning for Bluetooth devices\u2026</div>';

    const renderCandidates = async () => {
      try {
        const list = await call('chooser_candidates');
        if (!chooserOverlay) return;
        if (!list || !list.length) {
          bodyEl.innerHTML = '<div style="padding: 24px; text-align: center; color: #8a8a9a;">Scanning for Bluetooth devices\u2026</div>';
          return;
        }
        bodyEl.replaceChildren();
        for (const c of list) {
          const item = document.createElement('div');
          item.className = 'terra-chooser-item';
          const info = document.createElement('div');
          info.innerHTML = `<div class="terra-chooser-name">${c.name || 'Unnamed Device'}</div><div class="terra-chooser-sub">${c.kind} \u00b7 ${c.rssi} dBm</div>`;
          const btn = document.createElement('button');
          btn.className = 'terra-chooser-btn';
          btn.textContent = 'Pair';
          btn.onclick = async () => {
            window.hideChooserModal();
            try { await call('chooser_pick', { id: c.id }); } catch (e) { console.error('[terra-chooser]', e); }
          };
          item.append(info, btn);
          bodyEl.append(item);
        }
      } catch (e) { console.error('[terra-chooser]', e); }
    };

    chooserOverlay = document.createElement('div');
    chooserOverlay.className = 'terra-chooser-overlay';
    chooserOverlay.innerHTML = `
      <div class="terra-chooser-modal">
        <div class="terra-chooser-head">
          <h3>Pair Bluetooth Device</h3>
        </div>
      </div>
    `;
    const modal = chooserOverlay.querySelector('.terra-chooser-modal');
    const footEl = document.createElement('div');
    footEl.className = 'terra-chooser-foot';
    const cancelBtn = document.createElement('button');
    cancelBtn.className = 'terra-chooser-cancel';
    cancelBtn.textContent = 'Cancel';
    cancelBtn.onclick = async () => {
      window.hideChooserModal();
      try { await call('chooser_cancel'); } catch (e) { console.error('[terra-chooser]', e); }
    };
    footEl.append(cancelBtn);
    modal.append(bodyEl, footEl);

    document.body.append(chooserOverlay);
    renderCandidates();
    chooserPollTimer = setInterval(renderCandidates, 1000);
  };

  window.hideChooserModal = () => {
    if (chooserPollTimer) { clearInterval(chooserPollTimer); chooserPollTimer = null; }
    if (chooserOverlay) { chooserOverlay.remove(); chooserOverlay = null; }
  };

  window.__terra = window.__terra || {};
  window.__terra.showChooserModal = window.showChooserModal;
  window.__terra.hideChooserModal = window.hideChooserModal;

  const deviceFor = (info) => {
    let d = devices.get(info.id);
    if (!d) { d = new TerraBluetoothDevice(info); devices.set(info.id, d); }
    return d;
  };
  window.__terraCore = { call, log, devices, bus, toB64, toDV, deviceFor };

  Object.defineProperty(navigator, 'bluetooth', { value: bluetooth, configurable: true });
  log('shim-installed', { url: location.href });
})();
