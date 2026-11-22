// Terra TUI page bridge (Linux / Electron only).
//
// Stays completely dormant until TUI mode is active (TERRA_TUI=1 / TERRA_HEADLESS=1 or a
// `terra --tui` client connects). When active it does three things:
//   1. Keeps requestAnimationFrame ticking (~30 Hz) when the Electron window is hidden (or in
//      3D-off / 1fps mode) so Biketerra's simulation, BLE and ride broadcasts never stall.
//   2. Controls the 3D canvas renderer (`auto` / `off` / `1fps` / `full`): skips WebGPU/WebGL
//      draw calls and/or scales the canvas to 0.1x (min 64x36) at 1fps while keeping physics live.
//   3. Samples full route geometry (`prevPathNode.pathNode.list`), a high-res local curved road
//      ribbon (±120 m around the focal rider in heading-up local coords), and all riders'
//      positions/power/draft from `window.btApp.riderController` at ~20 Hz (`tui_route` / `tui_world`),
//      plus handles spectate focal-rider switching from the TUI.
(function () {
  'use strict';
  if (window.__terraTui) return;

  const boot = window.__TERRA_TUI_BOOT__ || {};
  const state = {
    tuiMode: Boolean(boot.tuiMode || boot.headless),
    clientCount: 0,
    windowHidden: Boolean(boot.headless),
    renderMode: 'auto', // 'auto' | 'off' | '1fps' | 'full'
  };

  const isActive = () => state.tuiMode || state.clientCount > 0;

  const effectiveRender = () => {
    if (!isActive()) return 'full';
    if (state.renderMode === 'auto') return state.windowHidden ? 'off' : 'full';
    if (state.renderMode === 'off') return 'off';
    if (state.renderMode === '1fps') return '1fps';
    // Even in 'full' mode, skip GPU draws while the window is hidden so WebGPU
    // getCurrentTexture() cannot stall on an unmapped swapchain.
    if (state.windowHidden) return 'off';
    return 'full';
  };

  const shouldPumpRaf = () => {
    if (!isActive()) return false;
    return state.windowHidden || document.hidden || effectiveRender() !== 'full';
  };

  // ---- 1. Keep page visible + rAF alive when hidden / 3D throttled ----

  try {
    const hiddenDesc = Object.getOwnPropertyDescriptor(Document.prototype, 'hidden');
    const visDesc = Object.getOwnPropertyDescriptor(Document.prototype, 'visibilityState');
    if (hiddenDesc && typeof hiddenDesc.get === 'function' && hiddenDesc.configurable) {
      Object.defineProperty(Document.prototype, 'hidden', {
        configurable: true,
        get() {
          if (isActive() && state.windowHidden) return false;
          return hiddenDesc.get.call(this);
        },
      });
    }
    if (visDesc && typeof visDesc.get === 'function' && visDesc.configurable) {
      Object.defineProperty(Document.prototype, 'visibilityState', {
        configurable: true,
        get() {
          if (isActive() && state.windowHidden) return 'visible';
          return visDesc.get.call(this);
        },
      });
    }
  } catch {}

  const origRAF = window.requestAnimationFrame.bind(window);
  const origCAF = window.cancelAnimationFrame.bind(window);
  const origSetTimeout = window.setTimeout.bind(window);

  // ~30 Hz simulation tick while hidden/lite: responsive drafting & attacks with tiny CPU.
  const PUMP_INTERVAL_MS = 33;
  let nextSyntheticId = 1;
  const pending = new Map();
  let pumpTimer = null;

  function schedulePump() {
    if (pumpTimer !== null) return;
    pumpTimer = origSetTimeout(() => {
      pumpTimer = null;
      if (!pending.size) return;
      if (!shouldPumpRaf()) {
        for (const [id, entry] of pending) {
          if (entry.nativeId == null) {
            attachNative(id, entry);
          }
        }
        return;
      }
      const batch = Array.from(pending.entries());
      pending.clear();
      const now = performance.now();
      for (const [, entry] of batch) {
        if (entry.nativeId != null) {
          try { origCAF(entry.nativeId); } catch {}
        }
        try {
          entry.cb(now);
        } catch (err) {
          try { console.error('[terra-tui] rAF callback error:', err); } catch {}
        }
      }
      if (pending.size && shouldPumpRaf()) {
        schedulePump();
      }
    }, PUMP_INTERVAL_MS);
  }

  function attachNative(id, entry) {
    entry.nativeId = origRAF((ts) => {
      if (!pending.has(id)) return;
      pending.delete(id);
      entry.cb(ts);
    });
  }

  function migratePendingToPump() {
    if (!pending.size) return;
    for (const [, entry] of pending) {
      if (entry.nativeId != null) {
        try { origCAF(entry.nativeId); } catch {}
        entry.nativeId = null;
      }
    }
    schedulePump();
  }

  window.requestAnimationFrame = function terraRaf(cb) {
    if (typeof cb !== 'function') return 0;
    const id = nextSyntheticId++;
    const entry = { cb, nativeId: null };
    pending.set(id, entry);
    if (shouldPumpRaf()) {
      schedulePump();
    } else {
      attachNative(id, entry);
    }
    return id;
  };

  window.cancelAnimationFrame = function terraCaf(id) {
    const entry = pending.get(id);
    if (!entry) return;
    pending.delete(id);
    if (entry.nativeId != null) {
      try { origCAF(entry.nativeId); } catch {}
    }
  };

  document.addEventListener('visibilitychange', () => {
    if (shouldPumpRaf()) migratePendingToPump();
  });

  // ---- 2. 3D Canvas Disabler / 0.1x @ 1fps Limiter ----

  const LITE_SCALE = 0.1;
  const MIN_CANVAS_W = 64;
  const MIN_CANVAS_H = 36;
  const ONE_FPS_MS = 1000;

  let gpuEng = null;
  let gpuCanvas = null;
  let origGpuResize = null;
  let origGpuRender = null;
  let lastGpuScaleMode = 'full';
  let lastRenderAt = 0;

  let glRenderer = null;
  let origGlRender = null;
  let origGlSetPixelRatio = null;
  let lastGlScaleMode = 'full';

  let featureHookedApp = null;
  let lastCanvasHidden = false;

  function getViewport(canvas) {
    const vw = window.innerWidth || (canvas && canvas.clientWidth) || 1280;
    const vh = window.innerHeight || (canvas && canvas.clientHeight) || 720;
    return { vw: Math.max(1, Math.round(vw)), vh: Math.max(1, Math.round(vh)) };
  }

  function liteDims(canvas) {
    const { vw, vh } = getViewport(canvas);
    return {
      vw,
      vh,
      tw: Math.max(MIN_CANVAS_W, Math.round(vw * LITE_SCALE)),
      th: Math.max(MIN_CANVAS_H, Math.round(vh * LITE_SCALE)),
    };
  }

  function syncCanvasVisibility(mode) {
    const canvas = gpuCanvas || (glRenderer && glRenderer.domElement) || document.querySelector('canvas');
    if (!canvas) return;
    const hide = mode === 'off';
    if (hide !== lastCanvasHidden) {
      lastCanvasHidden = hide;
      try {
        canvas.style.visibility = hide ? 'hidden' : '';
      } catch {}
    }
  }

  function applyWebGpuMode(eng, mode) {
    if (!eng || !origGpuResize) return;
    const canvas = (eng.device && eng.device.context && eng.device.context.canvas) || gpuCanvas;
    if (!canvas) return;
    gpuCanvas = canvas;
    syncCanvasVisibility(mode);

    if (mode === 'off' || mode === '1fps') {
      const { vw, vh, tw, th } = liteDims(canvas);
      if (canvas.width !== tw || canvas.height !== th || lastGpuScaleMode === 'full') {
        try {
          canvas.width = tw;
          canvas.height = th;
          canvas.style.width = `${vw}px`;
          canvas.style.height = `${vh}px`;
          origGpuResize.call(eng, tw, th);
        } catch {}
      }
      lastGpuScaleMode = mode;
    } else if (lastGpuScaleMode !== 'full') {
      lastGpuScaleMode = 'full';
      try {
        const { vw, vh } = getViewport(canvas);
        const dpr = Math.max(1, Number(window.devicePixelRatio) || 1);
        const rw = Math.round(vw * dpr);
        const rh = Math.round(vh * dpr);
        canvas.width = rw;
        canvas.height = rh;
        canvas.style.width = `${vw}px`;
        canvas.style.height = `${vh}px`;
        origGpuResize.call(eng, rw, rh);
        window.dispatchEvent(new Event('resize'));
      } catch {}
    }
  }

  function installWebGpu(eng) {
    if (!eng || typeof eng.render !== 'function' || typeof eng.resize !== 'function') return;
    if (gpuEng === eng) return;
    gpuEng = eng;
    gpuCanvas = (eng.device && eng.device.context && eng.device.context.canvas) || document.querySelector('canvas');
    origGpuResize = eng.resize;
    origGpuRender = eng.render;
    lastGpuScaleMode = 'full';

    eng.resize = function terraTuiResize(w, h) {
      const mode = effectiveRender();
      if (mode === 'off' || mode === '1fps') {
        const c = (this.device && this.device.context && this.device.context.canvas) || gpuCanvas;
        if (c) {
          gpuCanvas = c;
          const { vw, vh, tw, th } = liteDims(c);
          try {
            c.width = tw;
            c.height = th;
            c.style.width = `${vw}px`;
            c.style.height = `${vh}px`;
            lastGpuScaleMode = mode;
            return origGpuResize.call(this, tw, th);
          } catch {}
        }
      }
      return origGpuResize.call(this, w, h);
    };

    eng.render = function terraTuiRender(fs, rq) {
      const mode = effectiveRender();
      if (mode === 'off') {
        if (lastGpuScaleMode !== 'off') applyWebGpuMode(this, 'off');
        return;
      }
      if (mode === '1fps') {
        if (lastGpuScaleMode !== '1fps') applyWebGpuMode(this, '1fps');
        const now = performance.now();
        if (now - lastRenderAt < ONE_FPS_MS) return;
        lastRenderAt = now;
        return origGpuRender.call(this, fs, rq);
      }
      if (lastGpuScaleMode !== 'full') applyWebGpuMode(this, 'full');
      return origGpuRender.call(this, fs, rq);
    };

    applyWebGpuMode(eng, effectiveRender());
  }

  function applyWebGlMode(mode) {
    if (!glRenderer || !origGlSetPixelRatio) return;
    syncCanvasVisibility(mode);
    const { vw, vh } = getViewport(glRenderer.domElement);
    if (mode === 'off' || mode === '1fps') {
      if (lastGlScaleMode === 'full') {
        try {
          origGlSetPixelRatio.call(glRenderer, LITE_SCALE);
          if (typeof glRenderer.setSize === 'function') glRenderer.setSize(vw, vh);
        } catch {}
      }
      lastGlScaleMode = mode;
    } else if (lastGlScaleMode !== 'full') {
      lastGlScaleMode = 'full';
      try {
        origGlSetPixelRatio.call(glRenderer, Math.min(2, Number(window.devicePixelRatio) || 1));
        if (typeof glRenderer.setSize === 'function') glRenderer.setSize(vw, vh);
        window.dispatchEvent(new Event('resize'));
      } catch {}
    }
  }

  function installWebGl(renderer) {
    if (!renderer || typeof renderer.render !== 'function' || glRenderer === renderer) return;
    glRenderer = renderer;
    origGlRender = renderer.render;
    origGlSetPixelRatio = typeof renderer.setPixelRatio === 'function' ? renderer.setPixelRatio : null;
    lastGlScaleMode = 'full';

    if (origGlSetPixelRatio) {
      renderer.setPixelRatio = function terraTuiSetPR(pr) {
        const mode = effectiveRender();
        return origGlSetPixelRatio.call(this, mode === 'full' ? pr : LITE_SCALE);
      };
    }

    renderer.render = function terraTuiGlRender() {
      const mode = effectiveRender();
      if (mode === 'off') {
        if (lastGlScaleMode !== 'off') applyWebGlMode('off');
        return;
      }
      if (mode === '1fps') {
        if (lastGlScaleMode !== '1fps') applyWebGlMode('1fps');
        const now = performance.now();
        if (now - lastRenderAt < ONE_FPS_MS) return;
        lastRenderAt = now;
        return origGlRender.apply(this, arguments);
      }
      if (lastGlScaleMode !== 'full') applyWebGlMode('full');
      return origGlRender.apply(this, arguments);
    };

    applyWebGlMode(effectiveRender());
  }

  function probeEngine() {
    if (!isActive()) return;
    const app = window.btApp;
    if (!app) return;

    if (app.engine && typeof app.engine.render === 'function' && typeof app.engine.resize === 'function') {
      if (gpuEng !== app.engine) installWebGpu(app.engine);
    } else if (!app.engine && typeof app.addFeature === 'function' && featureHookedApp !== app) {
      featureHookedApp = app;
      try {
        app.addFeature((_scope, env) => ({
          frame() {
            try {
              if (env && env.engine && gpuEng !== env.engine) {
                installWebGpu(env.engine);
              }
            } catch {}
          },
        }));
      } catch {}
    }

    if (app.renderer && typeof app.renderer.render === 'function' && glRenderer !== app.renderer) {
      installWebGl(app.renderer);
    }
  }

  function syncEngineMode() {
    const mode = effectiveRender();
    if (gpuEng) applyWebGpuMode(gpuEng, mode);
    if (glRenderer) applyWebGlMode(mode);
    syncCanvasVisibility(mode);
  }

  // ---- 3. Route, Local Road Ribbon & All-Riders Telemetry ----

  const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, Number.isFinite(+v) ? +v : lo));
  const fin = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
  const normName = (s) => String(s || '').replace(/\s+/g, ' ').trim().toLowerCase();
  const r1 = (v) => Math.round(v * 10) / 10;
  const r2 = (v) => Math.round(v * 100) / 100;

  let cachedRoute = null; // { listRef, pts, dist, grades, totalLen, isLoop, n, payload }
  let sentRouteRef = null;
  const riderIdxHint = new Map();

  function readRouteName() {
    try {
      const title = String(document.title || '').replace(/\s*[-|·]\s*biketerra.*$/i, '').trim();
      if (title && !/^biketerra$/i.test(title) && !/^ride$/i.test(title)) return title;
      const u = new URL(location.href);
      return u.searchParams.get('name') || u.searchParams.get('route') || u.searchParams.get('event') || '';
    } catch {
      return '';
    }
  }

  function resolveRoute(rc) {
    try {
      const focal = rc.focalRider || rc.ego || (Array.isArray(rc.riders) ? rc.riders[0] : null);
      const prev = focal && focal.sim && focal.sim.prevPathNode;
      if (!prev) return cachedRoute;

      let list = null;
      if (prev.pathNode && Array.isArray(prev.pathNode.list) && prev.pathNode.list.length > 2) {
        list = prev.pathNode.list;
      } else if (Array.isArray(prev.list) && prev.list.length > 2) {
        list = prev.list;
      }
      if (!list || list.length < 3) return cachedRoute;
      if (cachedRoute && cachedRoute.listRef === list) return cachedRoute;

      const n = list.length;
      const pts = new Array(n);
      for (let i = 0; i < n; i++) {
        const node = list[i];
        const inner = (node && node.pathNode) ? node.pathNode : node;
        const p = inner && (inner.p || inner.pos || inner.position);
        if (!p || typeof p.x !== 'number') return cachedRoute;
        pts[i] = { x: p.x, y: Number.isFinite(p.y) ? p.y : 0, z: Number.isFinite(p.z) ? p.z : 0 };
      }

      const dist = new Float64Array(n);
      let minElev = pts[0].y;
      let maxElev = pts[0].y;
      let totalClimb = 0;
      for (let i = 1; i < n; i++) {
        const dx = pts[i].x - pts[i - 1].x;
        const dy = pts[i].y - pts[i - 1].y;
        const dz = pts[i].z - pts[i - 1].z;
        dist[i] = dist[i - 1] + Math.hypot(dx, dy, dz);
        if (pts[i].y < minElev) minElev = pts[i].y;
        if (pts[i].y > maxElev) maxElev = pts[i].y;
        if (dy > 0) totalClimb += dy;
      }
      const totalLen = dist[n - 1];
      if (!(totalLen > 1)) return cachedRoute;

      const loopGap = Math.hypot(pts[n - 1].x - pts[0].x, pts[n - 1].z - pts[0].z);
      const isLoop = n > 20 && loopGap < 80;

      const grades = new Float32Array(n);
      let back = 0, fwd = 0;
      for (let i = 0; i < n; i++) {
        const d = dist[i];
        while (back < i && d - dist[back] > 18) back++;
        while (fwd + 1 < n && dist[fwd] - d < 18) fwd++;
        const span = Math.max(1, dist[fwd] - dist[back]);
        grades[i] = clamp(((pts[fwd].y - pts[back].y) / span) * 100, -35, 35);
      }

      const MAX_PTS = 260;
      const step = Math.max(1, (n - 1) / (MAX_PTS - 1));
      const samples = [];
      let prevIdx = -1;
      for (let s = 0; s < MAX_PTS; s++) {
        const idx = s === MAX_PTS - 1 ? n - 1 : Math.min(n - 1, Math.round(s * step));
        if (idx === prevIdx) continue;
        prevIdx = idx;
        samples.push([r1(dist[idx]), r1(pts[idx].x), r1(pts[idx].z), r1(pts[idx].y), r1(grades[idx])]);
      }

      const payload = {
        name: readRouteName(),
        totalLen: r1(totalLen),
        isLoop,
        minElev: r1(minElev),
        maxElev: r1(maxElev),
        totalClimb: Math.round(totalClimb),
        points: samples,
      };

      riderIdxHint.clear();
      cachedRoute = { listRef: list, pts, dist, grades, totalLen, isLoop, n, payload };
      sentRouteRef = null;
      return cachedRoute;
    } catch {
      return cachedRoute;
    }
  }

  function wrapDist(route, d) {
    if (route.isLoop) {
      const m = d % route.totalLen;
      return m < 0 ? m + route.totalLen : m;
    }
    return clamp(d, 0, route.totalLen);
  }

  function sampleRouteAtDist(route, targetDist) {
    const { pts, dist, grades, n } = route;
    const d = wrapDist(route, targetDist);
    let lo = 0, hi = n - 1;
    while (lo + 1 < hi) {
      const mid = (lo + hi) >> 1;
      if (dist[mid] <= d) lo = mid;
      else hi = mid;
    }
    const segLen = dist[hi] - dist[lo];
    const t = segLen > 1e-6 ? clamp((d - dist[lo]) / segLen, 0, 1) : 0;
    return {
      x: pts[lo].x + (pts[hi].x - pts[lo].x) * t,
      y: pts[lo].y + (pts[hi].y - pts[lo].y) * t,
      z: pts[lo].z + (pts[hi].z - pts[lo].z) * t,
      grade: grades[lo] + (grades[hi] - grades[lo]) * t,
    };
  }

  function tangentAtDist(route, d, halfChord = 3.0) {
    const p0 = sampleRouteAtDist(route, d - halfChord);
    const p1 = sampleRouteAtDist(route, d + halfChord);
    const dx = p1.x - p0.x;
    const dz = p1.z - p0.z;
    const len = Math.hypot(dx, dz);
    if (len < 1e-5) return { tx: 0, tz: -1 };
    return { tx: dx / len, tz: dz / len };
  }

  function projectOnRoute(route, pos, key, prevNode) {
    if (!route || !pos || !Number.isFinite(pos.x) || !Number.isFinite(pos.z)) return null;
    const { pts, dist, grades, n } = route;
    let hint = prevNode && Number.isInteger(prevNode.index) && prevNode.index >= 0 && prevNode.index < n
      ? prevNode.index
      : (riderIdxHint.has(key) ? riderIdxHint.get(key) : 0);

    let bestIdx = hint;
    let bestD2 = Infinity;
    const lo = Math.max(0, hint - 30);
    const hi = Math.min(n - 1, hint + 30);
    for (let i = lo; i <= hi; i++) {
      const dx = pts[i].x - pos.x;
      const dz = pts[i].z - pos.z;
      const d2 = dx * dx + dz * dz;
      if (d2 < bestD2) { bestD2 = d2; bestIdx = i; }
    }
    if (bestD2 > 900) {
      const stride = n > 400 ? Math.floor(n / 200) : 1;
      for (let i = 0; i < n; i += stride) {
        const dx = pts[i].x - pos.x;
        const dz = pts[i].z - pos.z;
        const d2 = dx * dx + dz * dz;
        if (d2 < bestD2) { bestD2 = d2; bestIdx = i; }
      }
      if (stride > 1) {
        const fLo = Math.max(0, bestIdx - stride);
        const fHi = Math.min(n - 1, bestIdx + stride);
        for (let i = fLo; i <= fHi; i++) {
          const dx = pts[i].x - pos.x;
          const dz = pts[i].z - pos.z;
          const d2 = dx * dx + dz * dz;
          if (d2 < bestD2) { bestD2 = d2; bestIdx = i; }
        }
      }
    }
    riderIdxHint.set(key, bestIdx);

    let curDist = dist[bestIdx];
    const trySeg = (a, b) => {
      const vx = pts[b].x - pts[a].x;
      const vz = pts[b].z - pts[a].z;
      const len2 = vx * vx + vz * vz;
      if (len2 < 1e-4) return null;
      const t = ((pos.x - pts[a].x) * vx + (pos.z - pts[a].z) * vz) / len2;
      if (t < 0 || t > 1) return null;
      const px = pts[a].x + vx * t;
      const pz = pts[a].z + vz * t;
      return { d: dist[a] + (dist[b] - dist[a]) * t, d2: (pos.x - px) ** 2 + (pos.z - pz) ** 2 };
    };
    const cA = bestIdx > 0 ? trySeg(bestIdx - 1, bestIdx) : null;
    const cB = bestIdx + 1 < n ? trySeg(bestIdx, bestIdx + 1) : null;
    if (cA && (!cB || cA.d2 <= cB.d2)) curDist = cA.d;
    else if (cB) curDist = cB.d;

    return { dist: curDist, grade: grades[bestIdx] };
  }

  // Build local curved road ribbon (±120 m around focalDist) in heading-up local coords:
  // each entry is [centerRight_m, centerFwd_m, normRight, normFwd, grade_pct, relDist_m].
  function buildLocalRoad(route, focalDist) {
    if (!route || focalDist === null) return null;
    const anchor = sampleRouteAtDist(route, focalDist);
    const { tx, tz } = tangentAtDist(route, focalDist, 4.0);
    // Right-hand screen normal (X+ right, Y+ forward) for +Y up right-handed world
    const rx = -tz;
    const rz = tx;

    const toLocal = (wx, wz) => {
      const dx = wx - anchor.x;
      const dz = wz - anchor.z;
      return [dx * rx + dz * rz, dx * tx + dz * tz];
    };

    const RANGE = 120;
    const STEP = 4;
    const samples = [];
    for (let rel = -RANGE; rel <= RANGE; rel += STEP) {
      const d = focalDist + rel;
      if (!route.isLoop && (d < 0 || d > route.totalLen)) continue;
      const p = sampleRouteAtDist(route, d);
      const t = tangentAtDist(route, d, 2.5);
      // Local road right normal in world space
      const lrx = -t.tz;
      const lrz = t.tx;
      // Rotate local right normal into focal frame
      const nr = lrx * rx + lrz * rz;
      const nf = lrx * tx + lrz * tz;
      const [cx, cf] = toLocal(p.x, p.z);
      samples.push([r1(cx), r1(cf), r2(nr), r2(nf), r1(p.grade), rel]);
    }

    return { anchor, tx, tz, rx, rz, samples };
  }

  function parseRgb(v) {
    if (typeof v === 'string') {
      const m = v.trim().match(/^#?([0-9a-f]{6})/i) || v.trim().match(/^#?([0-9a-f]{3})$/i);
      if (m) {
        let h = m[1];
        if (h.length === 3) h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2];
        return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
      }
    }
    if (Array.isArray(v) && v.length >= 3) {
      return [clamp(v[0], 0, 255) | 0, clamp(v[1], 0, 255) | 0, clamp(v[2], 0, 255) | 0];
    }
    return null;
  }

  function riderIdOf(r, fallbackIdx) {
    const sim = r && r.sim;
    if (sim && sim.athleteId != null) return String(sim.athleteId);
    if (sim && sim.id != null) return String(sim.id);
    if (r && r.id != null) return String(r.id);
    return `bt-${fallbackIdx}`;
  }

  function riderNameOf(r, isEgo, id, feedById) {
    const cfg = (r && r.config) || {};
    const ud = (r && r.userData) || {};
    const first = String(ud.first_name != null ? ud.first_name : (cfg.first_name || '')).trim();
    const last = String(cfg.last_name != null ? cfg.last_name : (ud.last_name || '')).trim();
    const full = `${first} ${last}`.trim();
    if (full) return full;
    for (const c of [cfg.name, cfg.fullName, cfg.displayName, ud.name]) {
      if (c && String(c).trim()) return String(c).trim();
    }
    const f = feedById.get(id);
    if (f && f.name) return String(f.name);
    return isEgo ? 'You' : `Rider #${id}`;
  }

  function riderSpeedMs(r) {
    const sim = r && r.sim;
    if (!sim) return null;
    if (fin(sim.speed) !== null) return sim.speed;
    if (sim.velocity != null) {
      if (fin(sim.velocity) !== null) return sim.velocity;
      if (typeof sim.velocity === 'object') {
        const vx = fin(sim.velocity.x), vy = fin(sim.velocity.y) || 0, vz = fin(sim.velocity.z);
        if (vx !== null && vz !== null) return Math.hypot(vx, vy, vz);
      }
    }
    return null;
  }

  function helmetColor(r, id, rc) {
    const own = r && r.design && parseRgb(r.design.helmet_color);
    if (own) return own;
    if (rc && rc.humanFeed && rc.humanFeed.humans) {
      const h = rc.humanFeed.humans[id] || rc.humanFeed.humans[Number(id)];
      const hc = h && h.design && parseRgb(h.design.helmet_color);
      if (hc) return hc;
    }
    return null;
  }

  function invokeBackend(cmd, args) {
    if (!window.__TAURI_INTERNALS__ || typeof window.__TAURI_INTERNALS__.invoke !== 'function') return;
    window.__TAURI_INTERNALS__.invoke(cmd, args).catch(() => {});
  }

  let worldInFlight = false;
  let lastPath = location.href;
  let fastPolling = false;

  function sampleWorld() {
    if (!isActive() || worldInFlight) return;

    if (location.href !== lastPath) {
      lastPath = location.href;
      if (!/^\/(ride|spectate)/i.test(location.pathname)) {
        cachedRoute = null;
        sentRouteRef = null;
        riderIdxHint.clear();
      }
    }

    probeEngine();
    syncEngineMode();

    const bt = window.btApp;
    const rc = bt && (bt.riderController || bt.rideController);
    const canvas = gpuCanvas || (glRenderer && glRenderer.domElement) || document.querySelector('canvas');
    const canvasW = canvas ? (canvas.width || 0) : 0;
    const canvasH = canvas ? (canvas.height || 0) : 0;

    if (!rc) {
      if (fastPolling) restartPolling(false);
      worldInFlight = true;
      window.__TAURI_INTERNALS__
        ?.invoke('tui_world', {
          data: {
            btAppReady: false,
            renderMode: state.renderMode,
            effectiveRender: effectiveRender(),
            windowHidden: state.windowHidden,
            canvasW,
            canvasH,
            riders: [],
          },
        })
        .catch(() => {})
        .finally(() => { worldInFlight = false; });
      return;
    }

    // Switch to 20 Hz fast polling once btApp.riderController is live.
    if (!fastPolling) restartPolling(true);

    const route = resolveRoute(rc);
    if (route && route.listRef !== sentRouteRef) {
      sentRouteRef = route.listRef;
      invokeBackend('tui_route', { data: route.payload });
    }

    const feed = window.__terraCore && window.__terraCore.ride && window.__terraCore.ride.latest;
    const feedById = new Map();
    const feedByName = new Map();
    if (feed && Array.isArray(feed.riders)) {
      for (const fr of feed.riders) {
        if (!fr) continue;
        if (fr.id != null) feedById.set(String(fr.id), fr);
        if (fr.name) feedByName.set(normName(fr.name), fr);
      }
    }

    const seen = new Set();
    const rawRiders = [];
    const push = (r) => {
      if (!r || !r.sim || r._isTerraGhost || seen.has(r)) return;
      seen.add(r);
      rawRiders.push(r);
    };
    if (rc.focalRider) push(rc.focalRider);
    if (rc.ego) push(rc.ego);
    for (const r of (rc.riders || [])) push(r);

    // First pass: find focal rider's route distance to construct the heading-up local road frame
    const focalRider = rc.focalRider || rc.ego || null;
    let focalId = null;
    let egoId = null;
    let focalDist = null;
    if (focalRider && focalRider.sim) {
      const fid = riderIdOf(focalRider, 0);
      const fpos = focalRider.sim.midPosition || focalRider.sim.position;
      const fproj = route && fpos ? projectOnRoute(route, fpos, fid, focalRider.sim.prevPathNode) : null;
      if (fproj) focalDist = fproj.dist;
    }

    const localRoad = buildLocalRoad(route, focalDist);

    const out = [];
    for (let i = 0; i < rawRiders.length; i++) {
      const r = rawRiders[i];
      const sim = r.sim;
      const isFocal = r === rc.focalRider;
      const isEgo = Boolean(rc.ego && r === rc.ego);
      const id = riderIdOf(r, i);
      if (isFocal) focalId = id;
      if (isEgo) egoId = id;

      const name = riderNameOf(r, isEgo, id, feedById);
      const feedSt = feedById.get(id) || feedByName.get(normName(name)) || (isEgo && feed ? feed.ego : null);
      const pos = sim.midPosition || sim.position;
      const x = pos && fin(pos.x);
      const y = pos && fin(pos.y);
      const z = pos && fin(pos.z);

      const proj = route && pos ? projectOnRoute(route, pos, id, sim.prevPathNode) : null;
      const dist = proj ? r1(proj.dist) : (feedSt && fin(feedSt.distance));

      let rx = null, rf = null;
      if (localRoad && x !== null && z !== null) {
        const dx = x - localRoad.anchor.x;
        const dz = z - localRoad.anchor.z;
        rx = r2(dx * localRoad.rx + dz * localRoad.rz);
        rf = r2(dx * localRoad.tx + dz * localRoad.tz);
      }

      const power = fin(sim.power) ?? (feedSt && fin(feedSt.power));
      const massG = (r.config && fin(r.config.mass)) || (r.userData && fin(r.userData.weight));
      const wkg = (feedSt && fin(feedSt.wkg)) ?? (power !== null && massG ? r2((power * 1000) / massG) : null);
      const speed = riderSpeedMs(r) ?? (feedSt && fin(feedSt.speed));
      const cadence = (feedSt && fin(feedSt.cadence)) ?? fin(sim.cadence);
      const hr = (feedSt && fin(feedSt.heartRate)) ?? fin(sim.heartRate);
      const gap = feedSt && fin(feedSt.gap);
      const lap = (feedSt && fin(feedSt.lapCount)) ?? fin(sim.lapCount);
      const draft = fin(sim.draftFactor) ?? (isEgo && feed && feed.ego ? fin(feed.ego.draftFactor) : null);
      const isRow = (v) => v === 1 || v === '1' || v === 'row' || v === 'rowing';
      const sport = (
        isRow(feedSt && feedSt.sport) ||
        (isEgo && feed && (isRow(feed.sport) || isRow(feed.ego && feed.ego.sport))) ||
        isRow(sim.sport) ||
        isRow(r.sport) ||
        isRow(r.config && r.config.sport) ||
        isRow(r.userData && r.userData.sport)
      ) ? 1 : 0;

      let distDelta = null;
      if (focalDist !== null && dist !== null) {
        let dd = dist - focalDist;
        if (route && route.isLoop && route.totalLen > 0) {
          if (dd > route.totalLen * 0.5) dd -= route.totalLen;
          if (dd < -route.totalLen * 0.5) dd += route.totalLen;
        }
        distDelta = r1(dd);
      }

      out.push({
        id,
        name,
        isEgo,
        isFocal,
        x: x !== null ? r1(x) : null,
        y: y !== null ? r1(y) : null,
        z: z !== null ? r1(z) : null,
        rx,
        rf,
        dist,
        distDelta,
        grade: proj ? r1(proj.grade) : null,
        power: power !== null ? Math.round(power) : null,
        wkg,
        speed: speed !== null ? r2(speed) : null,
        cadence: cadence !== null ? Math.round(cadence) : null,
        hr: hr !== null ? Math.round(hr) : null,
        gap,
        lap,
        draft: draft !== null ? r2(draft) : null,
        sport,
        color: helmetColor(r, id, rc),
      });
    }

    worldInFlight = true;
    window.__TAURI_INTERNALS__
      ?.invoke('tui_world', {
        data: {
          btAppReady: true,
          focalId,
          egoId,
          renderMode: state.renderMode,
          effectiveRender: effectiveRender(),
          windowHidden: state.windowHidden,
          canvasW,
          canvasH,
          road: localRoad ? localRoad.samples : null,
          riders: out,
        },
      })
      .catch(() => {})
      .finally(() => { worldInFlight = false; });
  }

  // ---- 4. Spectate Focal-Rider Switching (from TUI Enter key) ----

  function wakeNearbyRiders(rc) {
    try {
      for (const r of (rc.riders || [])) {
        if (r && typeof r.updateCounter === 'number' && r.updateCounter > 0) {
          r.updateCounter = 0;
        }
      }
    } catch {}
  }

  function focusRider(targetId, targetName) {
    try {
      const bt = window.btApp;
      const rc = bt && (bt.riderController || bt.rideController);
      const wantId = targetId != null ? String(targetId) : '';
      const wantName = normName(targetName);

      if (wantName) {
        const rows = document.querySelectorAll('.hud-zone.zone-people .rider:not(.is-group), .riders-main .rider:not(.is-group), .rider:not(.is-group)');
        for (const row of rows) {
          const el = row.querySelector('.rider-name-btn, .rider-name');
          if (el && normName(el.textContent) === wantName) {
            const btn = row.querySelector('.rider-name-btn') || row.querySelector('button') || el;
            if (typeof btn.click === 'function') {
              btn.click();
              if (rc) wakeNearbyRiders(rc);
              return true;
            }
          }
        }
      }

      if (rc && typeof rc.setFocalRider === 'function') {
        const all = [];
        if (rc.focalRider) all.push(rc.focalRider);
        if (rc.ego) all.push(rc.ego);
        for (const r of (rc.riders || [])) all.push(r);
        for (const r of all) {
          if (!r || !r.sim) continue;
          const aid = r.sim.athleteId != null ? r.sim.athleteId : r.sim.id;
          if (wantId && aid != null && String(aid) === wantId) {
            rc.setFocalRider(aid);
            wakeNearbyRiders(rc);
            return true;
          }
          const cfg = r.config || {}, ud = r.userData || {};
          const full = normName(`${ud.first_name ?? cfg.first_name ?? ''} ${cfg.last_name ?? ud.last_name ?? ''}`);
          if (wantName && full === wantName && aid != null) {
            rc.setFocalRider(aid);
            wakeNearbyRiders(rc);
            return true;
          }
        }
      }
    } catch {}
    return false;
  }

  // ---- 5. Host control API (called by electron/main.js) ----

  let pollTimer = null;
  function restartPolling(fast) {
    fastPolling = fast;
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
    if (isActive()) {
      pollTimer = setInterval(sampleWorld, fast ? 50 : 300);
    }
  }

  function ensurePolling() {
    if (isActive() && pollTimer === null) {
      restartPolling(Boolean(window.btApp && (window.btApp.riderController || window.btApp.rideController)));
      sampleWorld();
    } else if (!isActive() && pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
      fastPolling = false;
    }
  }

  function setHostState(next) {
    if (!next || typeof next !== 'object') return;
    if (typeof next.tuiMode === 'boolean') state.tuiMode = next.tuiMode;
    if (typeof next.clientCount === 'number') state.clientCount = next.clientCount;
    if (typeof next.windowHidden === 'boolean') state.windowHidden = next.windowHidden;
    if (typeof next.renderMode === 'string') state.renderMode = next.renderMode;

    if (shouldPumpRaf()) {
      migratePendingToPump();
    }
    syncEngineMode();
    ensurePolling();
    if (isActive()) {
      if (cachedRoute) invokeBackend('tui_route', { data: cachedRoute.payload });
      sampleWorld();
    }
  }

  window.__terraTui = {
    setHostState,
    focusRider,
    getState: () => ({ ...state, effectiveRender: effectiveRender(), active: isActive() }),
  };

  if (isActive()) {
    ensurePolling();
  }
})();
