// ES2023+ polyfills for older WebViews (e.g. Chromium 109 on Android 13 where
// Array.prototype.toSorted/toReversed/with shipped in Chromium 110).
(() => {
  const def = (proto, name, fn) => {
    if (proto && !(name in proto)) {
      Object.defineProperty(proto, name, {
        value: fn,
        writable: true,
        configurable: true,
      });
    }
  };
  def(Array.prototype, 'toSorted', function (compareFn) {
    return Array.from(this).sort(compareFn);
  });
  def(Array.prototype, 'toReversed', function () {
    return Array.from(this).reverse();
  });
  def(Array.prototype, 'toSpliced', function (start, deleteCount, ...items) {
    const copy = Array.from(this);
    if (arguments.length === 0) return copy;
    if (arguments.length === 1) copy.splice(start);
    else copy.splice(start, deleteCount, ...items);
    return copy;
  });
  def(Array.prototype, 'with', function (index, value) {
    const len = this.length >>> 0;
    let idx = Math.trunc(Number(index)) || 0;
    if (idx < 0) idx += len;
    if (idx < 0 || idx >= len) throw new RangeError('Invalid index');
    const copy = Array.from(this);
    copy[idx] = value;
    return copy;
  });
  const TypedArrayProto = Object.getPrototypeOf(Int8Array.prototype);
  def(TypedArrayProto, 'toSorted', function (compareFn) {
    return new this.constructor(this).sort(compareFn);
  });
  def(TypedArrayProto, 'toReversed', function () {
    return new this.constructor(this).reverse();
  });
  def(TypedArrayProto, 'with', function (index, value) {
    const len = this.length >>> 0;
    let idx = Math.trunc(Number(index)) || 0;
    if (idx < 0) idx += len;
    if (idx < 0 || idx >= len) throw new RangeError('Invalid index');
    const copy = new this.constructor(this);
    copy[idx] = value;
    return copy;
  });
  if (typeof Promise.withResolvers !== 'function') {
    Promise.withResolvers = function () {
      let resolve, reject;
      const promise = new Promise((res, rej) => {
        resolve = res;
        reject = rej;
      });
      return { promise, resolve, reject };
    };
  }
  if (typeof Object.groupBy !== 'function') {
    Object.groupBy = function (items, callbackFn) {
      const obj = Object.create(null);
      let i = 0;
      for (const item of items) {
        const key = callbackFn(item, i++);
        if (key in obj) obj[key].push(item);
        else obj[key] = [item];
      }
      return obj;
    };
  }
  if (typeof Map.groupBy !== 'function') {
    Map.groupBy = function (items, callbackFn) {
      const map = new Map();
      let i = 0;
      for (const item of items) {
        const key = callbackFn(item, i++);
        let list = map.get(key);
        if (!list) {
          list = [];
          map.set(key, list);
        }
        list.push(item);
      }
      return map;
    };
  }
  if (typeof URL.canParse !== 'function') {
    URL.canParse = function (url, base) {
      try {
        new URL(url, base);
        return true;
      } catch {
        return false;
      }
    };
  }
})();

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
  const autoPaired = new Set(); // device ids already handed to the page on the current ride
  const pageDeviceIds = new Set(); // non-addon devices currently paired to the page

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
          // Wait for any in-flight ble_disconnect / unsubscribe from a previous ride to finish first.
          await (opTail.get(this.device.id) || Promise.resolve());
          const tree = await call('ble_connect', { id: this.device.id }).catch((e) => {
            autoPaired.delete(this.device.id);
            pageDeviceIds.delete(this.device.id);
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
      autoPaired.delete(this.device.id);
      pageDeviceIds.delete(this.device.id);
      if (!this.connected) return;
      trace('disconnect', { id: this.device.id, name: this.device.name });
      this.connected = false;
      this.services = null;
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
      autoPaired.delete(id);
      pageDeviceIds.delete(id);
      const d = devices.get(id);
      const wasConnected = !!(d && d.gatt.connected);
      if (d) {
        d.gatt.connected = false;
        d.gatt.services = null;
      }
      // Always emit ble:disconnected so native-ble sees OS drops even if they happen mid-subscribe
      // (before dev.gatt.connected flips to true).
      bus.emit('ble:disconnected', { deviceId: id, name: d ? d.name : null });
      if (wasConnected) {
        d.dispatchEvent(new Event('gattserverdisconnected'));
      }
    },
    autoConnected(id) {
      bus.emit('ble:autoconnected', { deviceId: id });
    },
  };

  let probingVirtual = false; // set by native bridge when probing addon requestDevice wrappers
  const bluetooth = {
    async getAvailability() { return true; },
    async getDevices() { return [...devices.values()]; },
    async requestDevice(options = {}) {
      if (probingVirtual) throw notFound('virtual probe');
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
      let reqFilters = filters;
      if (silent) {
        // auto-trainer (or a future equivalent) decides bike vs row and publishes
        // window.__terraMode + window.__terraPrefer = { bike: <namePrefix>, row: <namePrefix> }
        // so the remembered-device path picks the trainer matching the session. No addon
        // publishing = no extra filter = the historical best-ranked-remembered behavior.
        const mode = window.__terraMode || 'bike';
        const pref = window.__terraPrefer && window.__terraPrefer[mode];
        if (pref && !filters.some((f) => f.namePrefix === pref)) {
          reqFilters = filters.concat([{ namePrefix: pref }]);
          log('silent auto-pair prefers mode', { mode, namePrefix: pref });
        }
      }
      let info;
      // Only exclude devices that are actually connected (or connecting) to the page right now,
      // so a device that failed to connect or dropped from Biketerra while still OS-connected
      // can always be re-paired cleanly.
      const excludeIds = silent
        ? [...autoPaired].filter((id) => {
            const d = devices.get(id);
            return d && (d.gatt.connected || d.gatt._connecting);
          })
        : [];
      try { info = await call('ble_request_device', { filters: reqFilters, exclusionFilters, acceptAll, silent, owner, excludeIds }); }
      catch (e) { window.__terraSettled?.(false); throw notFound(e); }
      if (silent) { // auto-pair: never hand an already-connected device to the page twice
        const existing = devices.get(info.id);
        if (existing && (existing.gatt.connected || existing.gatt._connecting)) {
          window.__terraSettled?.(false);
          throw notFound('already paired by auto-pair');
        }
        window.__terraLastPaired = info.id;
      }
      if (!owner) {
        // Track manual page pairings too so auto-pair doesn't re-click for an already-paired device.
        pageDeviceIds.add(info.id);
        autoPaired.add(info.id);
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
  const baseRequestDevice = bluetooth.requestDevice;

  // ---- Native BLE bridge (Biketerra's official window.isNative / window.native API) ----
  // When active, Biketerra switches from Web Bluetooth chooser mode to native scan/auto-connect
  // mode: entering /ride (new ride, resumed ride, or Ride 2) immediately connects already-connected
  // and saved devices without opening any menu or clicking any DOM button.
  // Active on Electron desktop (window.__terraDesktop) and Android (window.__terraAndroid).
  const nativeMode = (!!window.__terraDesktop || !!window.__terraAndroid) && (() => {
    try { return localStorage.getItem('terra.nativeBle') !== '0'; } catch { return true; }
  })();

  if (nativeMode) {
    // Seed Biketerra's `saved-devices` synchronously BEFORE Biketerra's bundle evaluates it,
    // so devices remembered in Terra auto-connect as soon as they are discovered.
    try {
      const seedIds = Array.isArray(window.__terraRememberedIds) ? [...window.__terraRememberedIds] : [];
      seedIds.push('terra-zwift-ride-obc');
      const saved = JSON.parse(localStorage.getItem('saved-devices') || '[]');
      const seeded = JSON.parse(localStorage.getItem('terra.nativeSeeded') || '[]');
      const savedSet = new Set(Array.isArray(saved) ? saved : []);
      const seededSet = new Set(Array.isArray(seeded) ? seeded : []);
      let changed = false;
      for (const id of seedIds) {
        if (!seededSet.has(id)) {
          savedSet.add(id);
          seededSet.add(id);
          changed = true;
        }
      }
      if (changed) {
        localStorage.setItem('saved-devices', JSON.stringify([...savedSet]));
        localStorage.setItem('terra.nativeSeeded', JSON.stringify([...seededSet]));
      }
    } catch {}

    const nativeListeners = new Map(); // event -> Set<cb>
    const nativeConnected = new Set(); // deviceIds currently connected through window.native
    const nativeConnecting = new Set(); // deviceIds with an in-flight connect() call
    const knownNames = new Map();      // deviceId -> name
    const virtualDevices = new Map();  // deviceId -> mock BluetoothDevice from addon wrappers
    const virtualChars = new Map();    // `${id}|${svc}|${chr}` -> mock characteristic
    const virtualCharWired = new WeakSet();
    const virtualDevWired = new WeakSet();
    let scanPollTimer = null;
    let scanBurstTimers = [];
    let activeFilterUuids = [];
    let emittedInScan = new Set();

    const emitNative = (event, payload) => {
      const cbs = nativeListeners.get(event);
      if (!cbs) return;
      for (const cb of [...cbs]) {
        try { cb(payload); } catch (e) { console.error('[terra-native]', event, e); }
      }
    };

    const autoPairEnabled = () => {
      try { return localStorage.getItem('terra.autopair') !== '0'; } catch { return true; }
    };

    const matchesAutoTrainerPref = (name) => {
      const pref = window.__terraPrefer && window.__terraPrefer[window.__terraMode || 'bike'];
      if (!pref || !name) return true;
      const otherMode = (window.__terraMode || 'bike') === 'row' ? 'bike' : 'row';
      const otherPref = window.__terraPrefer && window.__terraPrefer[otherMode];
      if (otherPref && name.startsWith(otherPref) && !name.startsWith(pref)) return false;
      return true;
    };

    async function probeVirtualDevices() {
      if (!navigator.bluetooth || navigator.bluetooth.requestDevice === baseRequestDevice) return [];
      const found = [];
      const probes = [
        { filters: [{ services: ['d273f680-d548-419d-b9d1-fa0472345229'] }] }, // OBC shifter (zwift-ride)
        { filters: [{ services: ['00001826-0000-1000-8000-00805f9b34fb'] }] }, // FTMS trainer (wifi-trainer-bridge)
      ];
      for (const opts of probes) {
        probingVirtual = true;
        try {
          const dev = await navigator.bluetooth.requestDevice(opts);
          if (dev && dev.id) {
            virtualDevices.set(dev.id, dev);
            if (dev.name) knownNames.set(dev.id, dev.name);
            found.push(dev);
          }
        } catch {
          // Expected when no addon intercepts this service
        } finally {
          probingVirtual = false;
        }
      }
      return found;
    }

    async function pollScanCandidates() {
      try {
        const list = await call('ble_scan_candidates', { filterUuids: activeFilterUuids });
        for (const c of list || []) {
          if (c.name) knownNames.set(c.id, c.name);
          if (!emittedInScan.has(c.id)) {
            emittedInScan.add(c.id);
            log('native-ble', { discovered: c.name || c.id, connected: !!c.connected, connecting: !!c.connecting });
            emitNative('deviceDiscovered', { deviceId: c.id, deviceName: c.name ?? null });
          }
        }
      } catch (e) {
        log('native-ble', { scanPollError: String(e) });
      }
      const vdevs = await probeVirtualDevices();
      for (const v of vdevs) {
        if (!emittedInScan.has(v.id)) {
          emittedInScan.add(v.id);
          emitNative('deviceDiscovered', { deviceId: v.id, deviceName: v.name ?? null });
        }
      }
    }

    bus.on('ble:notify', ({ deviceId, characteristic, value }) => {
      if (!nativeConnected.has(deviceId)) return;
      if (virtualDevices.has(deviceId)) return; // virtual devices emit directly via characteristicvaluechanged
      const u8 = new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
      emitNative('notifyValue', { deviceId, characteristicUuid: characteristic, value: u8 });
    });

    bus.on('ble:disconnected', ({ deviceId }) => {
      emittedInScan.delete(deviceId);
      if (!nativeConnected.delete(deviceId)) return;
      log('native-ble', { dropout: deviceId });
      emitNative('dropout', { deviceId });
    });

    bus.on('ble:autoconnected', async ({ deviceId }) => {
      if (!deviceId || !autoPairEnabled()) return;
      if (nativeConnected.has(deviceId) || nativeConnecting.has(deviceId)) return;
      try {
        const list = await call('ble_devices_status', {});
        const d = (list || []).find((x) => x.id === deviceId);
        if (!d || d.owner || !d.remembered || !d.connected) return;
        if (!matchesAutoTrainerPref(d.name)) return;
        if (d.name) knownNames.set(d.id, d.name);
        emittedInScan.delete(d.id);
        log('native-ble', { autoConnected: d.name || d.id });
        emitNative('deviceDiscovered', { deviceId: d.id, deviceName: d.name ?? null });
      } catch {}
    });

    // When an addon connects its own device (e.g. Zwift Ride controller waking up),
    // re-probe for virtual devices (e.g. Terra Zwift Ride OBC) and advertise them.
    // Probe across a few delays because addons may finish characteristic subscribes
    // and handshakes (e.g. Zwift Ride "RideOn") 0.5–3 s after gatt.connect() resolves.
    bus.on('ble:connected', ({ deviceId }) => {
      if (nativeConnected.has(deviceId)) return;
      for (const delay of [300, 1000, 2500, 5000]) {
        setTimeout(async () => {
          const vdevs = await probeVirtualDevices();
          for (const v of vdevs) {
            if (!nativeConnected.has(v.id) && !nativeConnecting.has(v.id)) {
              log('native-ble', { discoveredVirtual: v.name || v.id });
              emitNative('deviceDiscovered', { deviceId: v.id, deviceName: v.name ?? null });
            }
          }
        }, delay);
      }
    });

    const propsListOf = (p) => {
      const out = [];
      if (p.read) out.push('read');
      if (p.write) out.push('write');
      if (p.writeWithoutResponse) out.push('writeWithoutResponse');
      if (p.notify) out.push('notify');
      if (p.indicate) out.push('indicate');
      return out;
    };

    window.isNative = true;
    window.native = {
      platform: window.__terraAndroid ? 'android' : 'linux',
      version: 'terra',
      listen(event, cb) {
        if (!nativeListeners.has(event)) nativeListeners.set(event, new Set());
        nativeListeners.get(event).add(cb);
        return () => nativeListeners.get(event)?.delete(cb);
      },
      async invoke(cmd, args = {}) {
        switch (cmd) {
          case 'is_fullscreen':
            return !!document.fullscreenElement;
          case 'set_fullscreen':
            if (args.fullscreen) {
              if (!document.fullscreenElement && document.body?.requestFullscreen) {
                await document.body.requestFullscreen().catch(() => {});
              }
            } else if (document.fullscreenElement && document.exitFullscreen) {
              await document.exitFullscreen().catch(() => {});
            }
            return null;

          case 'start_scan': {
            activeFilterUuids = Array.isArray(args.filterUuids) ? args.filterUuids : [];
            emittedInScan = new Set();
            log('native-ble', { cmd: 'start_scan', filters: activeFilterUuids.length });
            await call('ble_native_scan_start', {}).catch(() => {});
            if (scanPollTimer) clearInterval(scanPollTimer);
            for (const t of scanBurstTimers) clearTimeout(t);
            scanBurstTimers = [];
            // Start polling shortly after start_scan so Biketerra's initial get_connected_devices()
            // resolves first before deviceDiscovered events fire.
            scanBurstTimers.push(setTimeout(pollScanCandidates, 150));
            scanBurstTimers.push(setTimeout(pollScanCandidates, 450));
            scanBurstTimers.push(setTimeout(pollScanCandidates, 900));
            scanPollTimer = setInterval(pollScanCandidates, 1000);
            return null;
          }

          case 'stop_scan': {
            if (scanPollTimer) { clearInterval(scanPollTimer); scanPollTimer = null; }
            for (const t of scanBurstTimers) clearTimeout(t);
            scanBurstTimers = [];
            emittedInScan.clear();
            log('native-ble', { cmd: 'stop_scan' });
            await call('ble_native_scan_stop', {}).catch(() => {});
            return null;
          }

          case 'get_connected_devices': {
            if (window.__terraModeReady) {
              await Promise.race([
                Promise.resolve(window.__terraModeReady).catch(() => {}),
                new Promise((r) => setTimeout(r, 4000)),
              ]);
            }
            const out = [];
            if (autoPairEnabled()) {
              const statuses = await call('ble_devices_status', {}).catch(() => []);
              for (const d of statuses || []) {
                if (!d.remembered || d.owner || !(d.connected || d.connecting)) continue;
                if (!matchesAutoTrainerPref(d.name)) continue;
                if (d.name) knownNames.set(d.id, d.name);
                out.push({ deviceId: d.id, deviceName: d.name ?? null });
              }
              const vdevs = await probeVirtualDevices();
              for (const v of vdevs) {
                if (!out.some((x) => x.deviceId === v.id)) {
                  out.push({ deviceId: v.id, deviceName: v.name ?? null });
                }
              }
            }
            log('native-ble', { cmd: 'get_connected_devices', devices: out.map((x) => x.deviceName || x.deviceId) });
            return out;
          }

          case 'connect': {
            const deviceId = args.deviceId;
            const filterSet = new Set((args.filterUuids || []).map((u) => uuid(u).toLowerCase()));
            const vdev = virtualDevices.get(deviceId);
            if (vdev) {
              log('native-ble', { cmd: 'connect-virtual', deviceId, name: vdev.name });
              await vdev.gatt.connect();
              const svcs = await vdev.gatt.getPrimaryServices();
              const services = {};
              for (const s of svcs) {
                const su = uuid(s.uuid).toLowerCase();
                const charsObj = {};
                const chars = await s.getCharacteristics().catch(() => []);
                for (const c of chars) {
                  const cu = uuid(c.uuid).toLowerCase();
                  virtualChars.set(`${deviceId}|${su}|${cu}`, c);
                  charsObj[cu] = { props: propsListOf(c.properties || {}) };
                  if ((c.properties?.notify || c.properties?.indicate) && (filterSet.size === 0 || filterSet.has(cu))) {
                    if (!virtualCharWired.has(c)) {
                      virtualCharWired.add(c);
                      c.addEventListener('characteristicvaluechanged', () => {
                        if (!nativeConnected.has(deviceId) || !c.value) return;
                        const dv = c.value;
                        const u8 = new Uint8Array(dv.buffer, dv.byteOffset, dv.byteLength);
                        emitNative('notifyValue', { deviceId, characteristicUuid: cu, value: u8 });
                      });
                    }
                    await c.startNotifications().catch(() => {});
                  }
                }
                services[su] = charsObj;
              }
              if (!virtualDevWired.has(vdev)) {
                virtualDevWired.add(vdev);
                vdev.addEventListener('gattserverdisconnected', () => {
                  if (nativeConnected.delete(deviceId)) emitNative('dropout', { deviceId });
                });
              }
              nativeConnected.add(deviceId);
              return { deviceId, deviceName: vdev.name ?? null, services };
            }

            const tConnect0 = performance.now();
            log('native-ble', { cmd: 'connect', deviceId, name: knownNames.get(deviceId) ?? null });
            nativeConnecting.add(deviceId);
            try {
              await (opTail.get(deviceId) || Promise.resolve());
              const tree = await call('ble_connect', { id: deviceId, waitVisible: true });
              const connectMs = Math.round(performance.now() - tConnect0);
              if (!Array.isArray(tree) || tree.length === 0) {
                throw new Error('no GATT services discovered');
              }
              const deviceName = knownNames.get(deviceId) ?? devices.get(deviceId)?.name ?? null;
              call('ble_remember', { id: deviceId, name: deviceName }).catch(() => {});

              let dev = devices.get(deviceId);
              if (!dev) { dev = new TerraBluetoothDevice({ id: deviceId, name: deviceName }); devices.set(deviceId, dev); }
              else if (deviceName && !dev.name) { dev.name = deviceName; }

              const services = {};
              const hasFtms = tree.some((s) => uuid(s.uuid).toLowerCase() === '00001826-0000-1000-8000-00805f9b34fb');
              // Match Biketerra's driver selection (`Ht` in BjsG8Fg_.js): when FTMS (0x1826) is
              // present, Biketerra ignores Wahoo proprietary control (a026e005), CPS (2a63), CSC
              // (2a5b), and FE-C (6e40fec2). Subscribing to unused control/data characteristics
              // (especially Wahoo a026e005 alongside FTMS 2ad9) adds noise and can destabilize
              // KICKR firmware.
              const skipWhenFtms = new Set([
                'a026e005-0a7d-4ab3-97fa-f1500f9feb8b',
                '00002a63-0000-1000-8000-00805f9b34fb',
                '00002a5b-0000-1000-8000-00805f9b34fb',
                '6e40fec2-b5a3-f393-e0a9-e50e24dcca9e',
              ]);
              const toSubscribe = [];
              for (const s of tree) {
                const su = uuid(s.uuid).toLowerCase();
                const charsObj = {};
                for (const c of s.characteristics) {
                  const cu = uuid(c.uuid).toLowerCase();
                  charsObj[cu] = { props: propsListOf(c) };
                  if (
                    (c.notify || c.indicate) &&
                    (filterSet.size === 0 || filterSet.has(cu)) &&
                    !(hasFtms && skipWhenFtms.has(cu))
                  ) {
                    toSubscribe.push({ service: su, characteristic: cu });
                  }
                }
                services[su] = charsObj;
              }
              nativeConnected.add(deviceId);
              const tSub0 = performance.now();
              let subOk = 0;
              for (const sub of toSubscribe) {
                if (!nativeConnected.has(deviceId)) {
                  throw new Error('device disconnected during subscribe');
                }
                const doSub = () => gattOp(
                  deviceId,
                  () => call('ble_subscribe', { id: deviceId, service: sub.service, characteristic: sub.characteristic }),
                  false,
                  'subscribe ' + sub.characteristic,
                );
                try {
                  await doSub();
                  subOk++;
                } catch (e) {
                  const msg = String(e);
                  log('native-ble', { subscribeFailed: sub.characteristic, error: msg });
                  if (!nativeConnected.has(deviceId) || /disconnected|not connected/i.test(msg)) {
                    nativeConnected.delete(deviceId);
                    emittedInScan.delete(deviceId);
                    throw new Error('device disconnected during subscribe: ' + msg);
                  }
                  // One brief retry for transient BlueZ ATT errors (e.g. 0x0e) while still connected.
                  await new Promise((r) => setTimeout(r, 300));
                  if (!nativeConnected.has(deviceId)) {
                    emittedInScan.delete(deviceId);
                    throw new Error('device disconnected during subscribe');
                  }
                  try {
                    await doSub();
                    subOk++;
                  } catch (e2) {
                    const msg2 = String(e2);
                    log('native-ble', { subscribeRetryFailed: sub.characteristic, error: msg2 });
                    if (!nativeConnected.has(deviceId) || /disconnected|not connected/i.test(msg2)) {
                      nativeConnected.delete(deviceId);
                      emittedInScan.delete(deviceId);
                      throw new Error('device disconnected during subscribe: ' + msg2);
                    }
                  }
                }
              }
              if (!nativeConnected.has(deviceId) || (toSubscribe.length > 0 && subOk === 0)) {
                nativeConnected.delete(deviceId);
                emittedInScan.delete(deviceId);
                throw new Error('characteristic subscriptions failed');
              }
              const subMs = Math.round(performance.now() - tSub0);
              const totalMs = Math.round(performance.now() - tConnect0);
              // Expose the connected GATT server to addons only after subscriptions finish so
              // addons (e.g. realistic-gears) don't interleave writes mid-subscribe.
              dev.gatt.services = tree.map((s) => new RemoteService(dev, s));
              dev.gatt.connected = true;
              bus.emit('ble:paired', { deviceId, name: deviceName });
              bus.emit('ble:connected', { deviceId, name: deviceName });
              log('native-ble', {
                cmd: 'connected',
                deviceId,
                services: Object.keys(services).length,
                subs: subOk,
                connectMs,
                subMs,
                totalMs,
              });
              return { deviceId, deviceName, services };
            } finally {
              nativeConnecting.delete(deviceId);
            }
          }

          case 'disconnect': {
            const deviceId = args.deviceId;
            log('native-ble', { cmd: 'disconnect', deviceId });
            nativeConnected.delete(deviceId);
            emittedInScan.delete(deviceId);
            const vdev = virtualDevices.get(deviceId);
            if (vdev) {
              try { vdev.gatt.disconnect(); } catch {}
              return null;
            }
            const d = devices.get(deviceId);
            if (d) {
              d.gatt.connected = false;
              d.gatt.services = null;
            }
            await gattOp(deviceId, () => call('ble_disconnect', { id: deviceId }), false, 'disconnect').catch(() => {});
            bus.emit('ble:disconnected', { deviceId, name: d ? d.name : null });
            return null;
          }

          case 'read_value': {
            const { deviceId, serviceUuid, characteristicUuid } = args;
            const su = uuid(serviceUuid).toLowerCase();
            const cu = uuid(characteristicUuid).toLowerCase();
            const vchr = virtualChars.get(`${deviceId}|${su}|${cu}`);
            if (vchr) {
              const dv = await vchr.readValue();
              return new Uint8Array(dv.buffer, dv.byteOffset, dv.byteLength);
            }
            const b64 = await gattOp(
              deviceId,
              () => call('ble_read', { id: deviceId, service: su, characteristic: cu }),
              false,
              'read ' + cu,
            );
            const dv = toDV(b64);
            return new Uint8Array(dv.buffer, dv.byteOffset, dv.byteLength);
          }

          case 'write_value':
          case 'write_value_without_response': {
            const { deviceId, serviceUuid, characteristicUuid, value } = args;
            const withResponse = cmd === 'write_value';
            const su = uuid(serviceUuid).toLowerCase();
            const cu = uuid(characteristicUuid).toLowerCase();
            const vchr = virtualChars.get(`${deviceId}|${su}|${cu}`);
            if (vchr) {
              return withResponse ? vchr.writeValueWithResponse(value) : vchr.writeValueWithoutResponse(value);
            }
            const b64 = toB64(value);
            await gattOp(
              deviceId,
              () => call('ble_write', { id: deviceId, service: su, characteristic: cu, value: b64, withResponse }),
              true,
              'write ' + cu,
            );
            return null;
          }

          default:
            log('native-ble', { unknownCmd: cmd, args });
            return null;
        }
      },
    };
    log('native-ble', { installed: true, seeded: (window.__terraRememberedIds || []).length });
  }

  // ---- auto-pair: press Biketerra's own "Pair device..." button for remembered devices ----
  // Skipped when Biketerra's native BLE bridge (`nativeMode`) is active.
  if (!nativeMode) (() => {
    const enabled = () => { try { return localStorage.getItem('terra.autopair') !== '0'; } catch { return true; } };
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    const rideKey = () => location.pathname;
    let lastRideKey = null, completedRideKey = null, lastBtnEl = null;
    let running = false, timer = null, resumeTimer = null, needsRetry = false;
    let autoOpenCount = 0;
    const triedAutoOpenIds = new Set();

    const resetRideState = () => {
      // Unsubscribe/detach page-owned devices when leaving a ride so starting a second ride
      // performs a clean ble_connect + fresh notify stream (Rust keeps the OS BLE link warm
      // for remembered devices, so reconnect is instant). Addon-owned devices are untouched.
      for (const id of [...pageDeviceIds]) {
        const d = devices.get(id);
        if (d) {
          d.gatt.connected = false;
          d.gatt.services = null;
        }
        gattOp(id, () => call('ble_disconnect', { id }), false, 'disconnect').catch(() => {});
      }
      pageDeviceIds.clear();
      autoPaired.clear();
      triedAutoOpenIds.clear();
      completedRideKey = null;
      lastBtnEl = null;
      needsRetry = false;
      autoOpenCount = 0;
      if (resumeTimer) { clearTimeout(resumeTimer); resumeTimer = null; }
    };

    const syncRoute = () => {
      const onRide = location.pathname.startsWith('/ride');
      if (!onRide) {
        if (lastRideKey !== null || autoPaired.size > 0 || pageDeviceIds.size > 0) resetRideState();
        lastRideKey = null;
        return false;
      }
      const key = rideKey();
      if (lastRideKey !== key) {
        resetRideState();
        lastRideKey = key;
        // On a resumed ride, Biketerra often auto-closes the Devices menu immediately.
        // Schedule a check that reopens it briefly if remembered devices are ready at BLE level.
        scheduleResumeOpen(900);
      }
      return true;
    };

    const scheduleResumeOpen = (delayMs) => {
      if (resumeTimer) clearTimeout(resumeTimer);
      resumeTimer = setTimeout(() => {
        resumeTimer = null;
        maybeAutoOpenForResume();
      }, delayMs);
    };

    bus.on('ble:disconnected', (e) => {
      if (e && e.deviceId) {
        autoPaired.delete(e.deviceId);
        pageDeviceIds.delete(e.deviceId);
        triedAutoOpenIds.delete(e.deviceId);
      }
      needsRetry = true;
      lastBtnEl = null;
      if (!timer) timer = setTimeout(check, 250);
    });

    bus.on('ble:autoconnected', (e) => {
      const id = e && e.deviceId;
      if (!id || !enabled() || !location.pathname.startsWith('/ride')) return;
      if (autoPaired.has(id) && devices.get(id)?.gatt?.connected) return;
      triedAutoOpenIds.delete(id);
      needsRetry = true;
      lastBtnEl = null;
      if (findBtn()) {
        if (!timer) timer = setTimeout(check, 150);
      } else {
        scheduleResumeOpen(350);
      }
    });

    const isVisible = (el) => {
      if (!el || !el.isConnected) return false;
      const r = el.getBoundingClientRect();
      if (r.width <= 0 || r.height <= 0) return false;
      try {
        const cs = getComputedStyle(el);
        if (cs.display === 'none' || cs.visibility === 'hidden' || cs.opacity === '0') return false;
      } catch {}
      return true;
    };

    const findBtn = () => {
      let best = null;
      for (const el of document.querySelectorAll('button, [role="button"], a, div, span')) {
        const t = (el.innerText || '').trim().toLowerCase();
        if (!t.startsWith('pair device') || t.length > 40) continue;
        if (!best || best.contains(el)) best = el; // prefer the innermost match
      }
      const btn = best && (best.closest('button, [role="button"], a') || best);
      return isVisible(btn) ? btn : null;
    };

    const findDevicesTab = () => {
      for (const el of document.querySelectorAll('button, [role="tab"], [role="button"]')) {
        if (!isVisible(el)) continue;
        const t = (el.innerText || '').trim().toLowerCase();
        if (t === 'devices') return el;
      }
      return null;
    };

    const hasModalPrompt = () => {
      for (const el of document.querySelectorAll('button, [role="button"]')) {
        if (!isVisible(el)) continue;
        const t = (el.innerText || '').trim().toLowerCase();
        if (t === 'resume' || t === 'resume ride' || t === 'discard') return true;
      }
      return false;
    };

    const sendKey = (key, code, keyCode) => {
      const target = document.body || document.documentElement;
      const init = { key, code, keyCode, which: keyCode, bubbles: true, cancelable: true, composed: true };
      target.dispatchEvent(new KeyboardEvent('keydown', init));
      target.dispatchEvent(new KeyboardEvent('keyup', init));
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

    async function waitForBtn(ms) {
      const deadline = Date.now() + ms;
      while (Date.now() < deadline) {
        const b = findBtn();
        if (b) return b;
        await sleep(50);
      }
      return findBtn();
    }

    async function maybeAutoOpenForResume() {
      if (running || !enabled() || !syncRoute()) return;
      const key = rideKey();
      if (autoOpenCount >= 3) return;
      if (document.querySelector('.terra-overlay, .terra-chooser-overlay')) return;
      const ae = document.activeElement;
      if (ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA' || ae.isContentEditable)) return;
      if (hasModalPrompt()) {
        // Pre-ride Resume/Discard prompt is still up; wait until the user answers it.
        scheduleResumeOpen(1000);
        return;
      }
      if (findBtn()) {
        if (!timer) timer = setTimeout(check, 100);
        return;
      }
      const statuses = await call('ble_devices_status', {}).catch(() => []);
      if (running || !syncRoute() || rideKey() !== key) return;
      const ready = (statuses || []).filter((d) =>
        d.remembered && !d.owner && d.connected &&
        !triedAutoOpenIds.has(d.id) &&
        !(devices.get(d.id)?.gatt?.connected) &&
        !(devices.get(d.id)?.gatt?._connecting)
      );
      if (!ready.length) return;
      if (findBtn()) {
        run(key, { closeAfter: false });
        return;
      }

      autoOpenCount++;
      for (const d of ready) triedAutoOpenIds.add(d.id);
      log('auto-pair resume-menu', { action: 'open', ready: ready.map((d) => d.name || d.id) });
      sendKey('m', 'KeyM', 77);
      let btn = await waitForBtn(550);
      if (!btn) {
        const tab = findDevicesTab();
        if (tab) {
          press(tab);
          btn = await waitForBtn(400);
        }
      }
      if (!btn) {
        const menuBtn = [...document.querySelectorAll('button[aria-label*="menu" i], button[title*="menu" i], [role="button"][aria-label*="menu" i]')]
          .find((el) => isVisible(el));
        if (menuBtn) {
          press(menuBtn);
          btn = await waitForBtn(500);
          if (!btn) {
            const tab = findDevicesTab();
            if (tab) {
              press(tab);
              btn = await waitForBtn(400);
            }
          }
        }
      }
      log('auto-pair resume-menu', { opened: !!btn });
      if (!btn || running || rideKey() !== key) return;
      await run(key, { closeAfter: true });
    }

    async function run(triggerKey, opts = {}) {
      const closeAfter = !!opts.closeAfter;
      if (running || !enabled()) return;
      if (!syncRoute() || rideKey() !== triggerKey) return;
      running = true;
      let pairedAny = false;
      let buttonDisappeared = false;
      let exhausted = false;
      try {
        // Give auto-trainer a bounded moment to decide bike vs row (it publishes
        // window.__terraModeReady; worst case is its ~3 s "is the Ride controller
        // stowed?" probe — a bike ride resolves as soon as the controller links).
        // Absent = no addon, press as before.
        if (window.__terraModeReady) {
          const how = await Promise.race([
            Promise.resolve(window.__terraModeReady).then(() => 'ready').catch(() => 'error'),
            sleep(12000).then(() => 'timeout'),
          ]);
          log('auto-pair waited for mode', { how, mode: window.__terraMode || null });
        }
        let emptyRetries = 0;
        const attemptsById = new Map();
        for (let i = 0; i < 6; i++) {
          if (rideKey() !== triggerKey) break;
          if (document.querySelector('.terra-chooser-overlay, .terra-overlay')) break;
          const b = findBtn();
          if (!b) {
            buttonDisappeared = true;
            break;
          }
          lastBtnEl = b;
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
          if (ok) {
            pairedAny = true;
            emptyRetries = 0;
            const pairedId = window.__terraLastPaired;
            const seenCount = (attemptsById.get(pairedId) || 0) + 1;
            attemptsById.set(pairedId, seenCount);
            await waitConnected(pairedId, 10000); // let the page finish connecting first
            await sleep(600); // give Biketerra time to subscribe before trying the next device
            if (seenCount >= 2 && !(devices.get(pairedId)?.gatt?.connected)) {
              log('auto-pair', { stopped: 'device rejected twice', id: pairedId });
              exhausted = true;
              break;
            }
            continue;
          }
          // On a cold start, the BLE scan may still be discovering/connecting remembered
          // devices when /ride first opens: retry while the Pair panel stays open.
          const anyPageConnected = [...pageDeviceIds].some((id) => devices.get(id)?.gatt?.connected);
          if (!closeAfter && !pairedAny && !anyPageConnected && emptyRetries < 6 && findBtn()) {
            emptyRetries++;
            await sleep(1500);
            i--;
            continue;
          }
          exhausted = true;
          break;
        }
        const anyPageConnectedNow = [...pageDeviceIds].some((id) => devices.get(id)?.gatt?.connected);
        if (exhausted && !buttonDisappeared && (pairedAny || anyPageConnectedNow)) {
          completedRideKey = triggerKey;
          needsRetry = false;
        } else if (exhausted && !buttonDisappeared && emptyRetries >= 6) {
          // Stay quiet for this button instance, but allow auto-pair again if the panel
          // closes and reopens later (e.g. after powering on a trainer/HR strap).
          needsRetry = false;
        }
        if (closeAfter && rideKey() === triggerKey && findBtn()) {
          log('auto-pair resume-menu', { action: 'close', pairedAny });
          sendKey('Escape', 'Escape', 27);
        }
      } finally {
        running = false;
        silentNext = false;
        if (needsRetry && rideKey() === triggerKey && findBtn()) {
          // A device finished auto-connecting (or disconnected) while we were mid-run.
          if (!timer) timer = setTimeout(check, 250);
        } else if (buttonDisappeared && !closeAfter && rideKey() === triggerKey) {
          // The Pair panel closed automatically (e.g. resumed ride) before finishing:
          // check shortly whether any remembered BLE device still needs pairing.
          scheduleResumeOpen(600);
        }
      }
    }

    const check = () => {
      timer = null;
      if (!syncRoute() || running) return;
      if (document.querySelector('.terra-chooser-overlay, .terra-overlay')) return;
      const b = findBtn();
      if (!b) {
        lastBtnEl = null;
        return;
      }
      const key = rideKey();
      const alreadyDone = completedRideKey === key && !needsRetry;
      if (alreadyDone) return;
      if (b === lastBtnEl && !needsRetry) return;
      lastBtnEl = b;
      setTimeout(() => run(key), 150);
    };
    addEventListener('popstate', () => { if (!timer) timer = setTimeout(check, 150); });
    addEventListener('hashchange', () => { if (!timer) timer = setTimeout(check, 150); });
    new MutationObserver(() => { if (!timer) timer = setTimeout(check, 100); })
      .observe(document, { childList: true, subtree: true, characterData: true });
  })();

  // In-page Bluetooth device chooser modal for Android & Webview contexts
  let chooserOverlay = null, chooserPollTimer = null;

  window.showChooserModal = () => {
    if (chooserOverlay) return;
    const CSS = `
.terra-chooser-overlay { position: fixed; inset: 0; z-index: 2147483647; display: flex; align-items: center; justify-content: center;
  background: rgba(0,0,0,.72); font: 14px system-ui, sans-serif; }
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
  // ---- ride broadcast data ----
  // While riding or spectating, Biketerra broadcasts the ride state on
  // BroadcastChannel('biketerra-ride') at ~2Hz (biketerra.com/help-center/live-broadcast-data).
  // Relay every frame to the Rust backend (it keeps the latest + a few minutes of history),
  // and emit it on the shared bus so addons can listen with zero round-trip.
  const ride = { latest: null, updatedAt: 0 };
  let lastPushErr = 0;
  if (typeof BroadcastChannel === 'function') {
    try {
      ride.channel = new BroadcastChannel('biketerra-ride');
      ride.channel.onmessage = (e) => {
        ride.latest = e.data;
        ride.updatedAt = Date.now();
        bus.emit('biketerra:ride', e.data);
        call('ride_push', { data: e.data }).catch((err) => {
          const now = Date.now();
          if (now - lastPushErr > 5000) { lastPushErr = now; log('ride-push', String(err)); }
        });
      };
    } catch (e) { log('ride-channel', String(e)); ride.channel = null; }
  }

  window.__terraCore = {
    call, log, devices, bus, toB64, toDV, deviceFor, ride,
    requestDevice: (opts) => bluetooth.requestDevice(opts),
  };

  Object.defineProperty(navigator, 'bluetooth', { value: bluetooth, configurable: true });
  log('shim-installed', { url: location.href });
})();
