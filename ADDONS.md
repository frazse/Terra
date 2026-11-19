# Terra addons

An addon is a folder with a `manifest.json`. Terra scans, in order:
`$TERRA_ADDONS_DIR` (if set), otherwise `~/.local/share/terra/addons/` and `./addons/`.
Folders starting with `_` or `.` are ignored. The folders are rescanned on every page load,
so edit an addon and press **F5** (or Ctrl+R) in the Terra window. No rebuild.

**Trust model:** addons run inside the biketerra.com page with the page's privileges (like
Tampermonkey userscripts), including the BLE commands. Only install addons you trust.

## Turning addons on and off
Click the **cog** next to Biketerra's theme toggle: every addon found in the folders is listed with a switch.
Changes apply immediately (the addon is started or cleaned up) and are remembered in
`~/.local/share/terra/addon-data/.addons-state.json`. The green dot means it is running now.

## manifest.json
    {
      "id": "my-addon",            // optional, defaults to the folder name ([A-Za-z0-9._-])
      "name": "My addon",
      "version": "0.1.0",
      "matches": ["https://biketerra.com/ride*"],   // `*` wildcards, matched against the full URL
      "js": ["main.js"],           // concatenated, run as the body of `async function (terra) {}`
      "css": ["style.css"],
      "description": "shown in the cog menu",
      "enabled": true      // default state; the cog menu overrides it
    }
The addon starts when the URL matches (also after SPA navigation) and is cleaned up when it stops matching.

## The `terra` object
| API | |
|---|---|
| `terra.id / name / version` | manifest info |
| `terra.log(...)` | prints `[shim] addon:<id>: ...` in the terminal |
| `terra.onCleanup(fn)` | run when the addon stops (remove your DOM, timers) |
| `terra.onNavigate(fn)` | called with the new URL on SPA navigation |
| `terra.waitFor(selector, {timeout, root})` | Promise of the element once it exists |
| `terra.storage.get(k, default) / set(k, v) / remove(k)` | async, JSON values, persisted by Rust per addon |
| `terra.bus.on(topic, cb) / emit(topic, data)` | in-page event bus shared by all addons |
| `terra.ble.onNotify(cb)` | `{deviceId, name, characteristic, value: DataView}` for every notification the page subscribed to |
| `terra.ble.onPaired / onConnect / onDisconnect(cb)` | device lifecycle events |
| `terra.ble.request(options)` | pair a device *for this addon* (always shows the chooser). It is remembered for the addon only, so the page's own auto-pair leaves it alone |
| `terra.ble.setShared(on)` | let the page (not only this addon) use the devices this addon paired, e.g. so Biketerra's own Zwift Ride support can take the controller. Resets on every page load, so set it each time |
| `terra.ble.remembered()` | devices this addon paired earlier, as `BluetoothDevice`s, no UI. Call `device.gatt.connect()` on them |
| `terra.ble.devices()` | `[{id, name, connected}]` known to the page |
| `terra.ride.onRide(cb)` | live ride data, ~2Hz (see [Live ride data](#live-ride-data)). Returns a cleanup fn |
| `terra.ride.latest()` | the newest broadcast frame (in-page, immediate — works even before any round-trip) |
| `terra.ride.history(limit?)` | async, the last ~5 min of frames from the Rust backend (or `limit` of them), oldest to newest |
| `terra.keys.down / up / tap(key, ms?, {target?})` | synthetic KeyboardEvents (default target: active element) |
| `terra.ui.addStyle(css)` | injects CSS (CSP-safe), removed on cleanup |
| `terra.udp.send(target, payload)` | fire-and-forget UDP datagram to `host` or `ip:port` (≤ 65507 bytes); `target` may contain `{{setting:KEY}}` placeholders; resolves to bytes sent. Great for reaching a bike computer on the local Wi-Fi |
 | `terra.tcp.connect(host, port, timeoutMs?)` | open a persistent TCP stream; resolves to a connection id. `host` may contain `{{setting:KEY}}` placeholders; `timeoutMs` 0 = 3 s |
 | `terra.tcp.send(conn, bytes)` | write raw bytes (a `Uint8Array` or `DataView`) onto the connection; resolves to bytes written |
 | `terra.tcp.close(conn)` | close the connection (sends EOF to the peer) |
 | `terra.tcp.list()` | this addon's open connections, `[{conn, peer}]` (diagnostics) |
 | `terra.tcp.onFrame(cb)` | inbound bytes: `{conn, bytes: DataView}`. The page is the only door in and out; framing is the addon's job |
 | `terra.tcp.onState(cb)` | lifecycle: `{conn, state: 'connected' | 'closed' | 'error', reason?, peer?}` |
| `terra.perf.stats()` | async, hardware & app performance snapshot: `{platform, cpuPct, cpuTempC, appCpuPct, gpuPct, gpuTempC, gpuVramUsedMb, gpuVramTotalMb, memUsedMb, memTotalMb, thermalStatus, batteryTempC, gpuProcCpuPct?, rendererCpuPct?, appMemMb?}` (unavailable fields are `null`) |

`navigator.bluetooth` also works inside addons (Terra's shim), so addons can pair and talk to their
own devices with the normal Web Bluetooth API. Remembered devices pair instantly, new ones open the chooser.

## Live ride data
While riding or spectating, Biketerra broadcasts the ride state at ~2Hz
(see <https://biketerra.com/help-center/live-broadcast-data>). Terra listens on that channel, relays
every frame to the Rust backend (which keeps the latest frame plus ~5 min of history), and emits each
frame on the bus (topic `biketerra:ride`). Addons use `terra.ride.onRide(cb)` for live updates,
`terra.ride.latest()` for the newest frame, and `terra.ride.history(limit?)` for a window of past
frames. Frame shape:

    {
      "ego":     { speed, speedAvg, power, powerAvg, wkg, cadence, heartRate, distance,
                   distanceRemaining, climb, elevation, grade, lat, lng, ... },
      "riders":  [{ id, name, power, wkg, speed, cadence, heartRate, gap, lapCount }, ...],
      "timestamp": 1776951073587
    }

## Drawing into the game scene (advanced)

> **Not part of the `terra` API.** Everything the `terra` object exposes above is stable Terra
> surface. This section reaches into **Biketerra.com's in-page engine**, like a userscript running
> with the page's privileges (the trust model above). It is a **reference for the current deploy,
> not a guaranteed API**: the names/shapes below come from the live game and can change between
> updates. When something stops matching, trust the real addons — `nameplates` and `route-sign`
> (committed, in `addons/`) and the local sandboxes `scene-probe` / `bike-preview` (see AGENTS.md §6) —
> over this prose. The local `nameplates3d` sandbox (per-rider 3D nameplates, textured
> geometry — §C below) is the reference for the textured path; it stays out of `addons/` on purpose.

Both techniques need one thing first: **`window.btApp`** — the game manager that owns
`addFeature`, `riderController`, and the render loop. The game keeps it inside a module closure,
so Terra publishes it for you: the Electron host sets a CDP conditional breakpoint on the game's
input handler whose condition is `window.btApp = this, false` (it runs `this` onto the page, never
pauses). Disable with **`TERRA_NO_BT_BREAKPOINT=1`**. Two consequences:

- **It appears late** (on the game's first ticks), so **poll** for it — don't assume it on boot.
- **Terra owns the debugger while attached**, so the page's own DevTools can't connect (Chromium
  allows one debugger client per target). On-page diagnostics therefore have to be DOM (a little
  status pill), not DevTools.

Wait for it, then hook the render loop. `remove` is the cleanup fn → wire it to `terra.onCleanup(remove)`:

    function waitForBT(ms = 20000) {
      const t0 = performance.now();
      return new Promise((res, rej) => {
        const poke = () => (window.btApp && typeof window.btApp.addFeature === 'function')
            ? res(window.btApp)
            : (performance.now() - t0 > ms ? rej(new Error('btApp not found')) : requestAnimationFrame(poke));
        requestAnimationFrame(poke);
      });
    }
    const bt = await waitForBT();

    const remove = bt.addFeature((scope, env) => ({
      frame(frameState, renderQueue) { /* runs every rendered frame */ }
    }));

`env` exposes the engine (`env.engine`, `env.tiles`, `env.budget`); `frameState` exposes
`frameState.camera`, `frameState.origin`, `frameState.rig`; `renderQueue.draws` is the live
per-frame draw list (the engine rebuilds it **every frame**, so anything you add must be re-added
each frame).

### A. 2D overlays anchored over the 3D riders (e.g. `nameplates`)

Draw a DOM node that tracks a rider in screen space, using the game's own camera — so it always
lines up with the 3D riders. Data + math (all per frame):

- World position: `bt.riderController.focalRider.sim.midPosition` (the watched rider) and
  `bt.riderController.riders[i].sim.midPosition` (everyone else); `{ x, y, z }` world metres, +Y up.
  Skip riders with `rider.sim.curLod === 0` (the game culled them).
- Camera matrix: `frameState.camera.viewProj` is a **16-element column-major** `P·V`.
  `frameState.origin = [ox, oz]` is the game's floating-origin offset — **subtract it from world X
  and Z before multiplying**. In first person `frameState.rig.hideFocalRider` is true, and the
  focal rider isn't on screen — hide your own plate for it too.
- World → screen (drop the plate unless the clip-W `w` is comfortably past the near plane and the
  point is within the NDC frustum):

      const [ox, oz] = frameState.origin;
      const m = frameState.camera.viewProj;                 // column-major
      const rx = p.x - ox, ry = p.y + LIFT, rz = p.z - oz;  // LIFT = plate height above the rider
      const w  = m[3]*rx + m[7]*ry + m[11]*rz + m[15];      // clip W (perspective divide)
      if (w <= 0.15 || !Number.isFinite(w)) { hide; return; }
      const nx = (m[0]*rx + m[4]*ry + m[8]*rz  + m[12]) / w;
      const ny = (m[1]*rx + m[5]*ry + m[9]*rz  + m[13]) / w;
      if (Math.abs(nx) > 1.10 || Math.abs(ny) > 1.10) { hide; return; }  // beyond the horizon
      const sx = (nx * 0.5 + 0.5) * window.innerWidth;
      const sy = (1 - (ny * 0.5 + 0.5)) * window.innerHeight;
      const sc = Math.max(0.5, Math.min(1.6, 10 / Math.max(10, w)));     // scale falloff with depth

- Position the node on a full-viewport `pointer-events:none` overlay so its **bottom-centre**
  sits at `(sx, sy)` (anchor just above the rider's head, via `LIFT` above `py`):

      el.style.transform =
        'translate3d(' + sx + 'px,' + sy + 'px,0) translate(-50%,-100%) scale(' + sc + ')';

`nameplates` is a complete, shippable example (labels, per-rider colour from
`rider.design.helmet_color`, range gating, cleanup-on-leave).

### B. Inject real 3D geometry into the scene (bikes, signs, custom meshes)

A DOM billboard can't be depth-tested against the world. To put *rendered* 3D into the scene,
hand the engine a draw entry into its own queue, using the same **misc pool** the game uses for
dynamic geometry. `route-sign` (a quad) is the minimal committed example; the local `bike-preview`
sandbox does the same but feeds it the game's full 5 202-vertex bike mesh (its skin data ignored — raw static geometry). In order:

1. **Wait for the pool.** On the first `frame`, read `env.engine.pools`; the dynamic pool is
   `env.engine.pools.misc`, identifiable by having both `slabs` (a `Set` of shared geometry) and a
   `format`. (If those names have changed, that's exactly what the `scene-probe` sandbox is for.)
2. **Read the vertex layout from `misc.format.buffers`** — an array of buffers, each with
   `attributes: [{ field, components, ... }]`. The `field` names (position/normal/uv/…) are
   **hashed per deploy** — read them live, never hard-code them. Typically position (3f), normal
   (3f), uv (2f).
3. **`handle = misc.allocate(scope, vertexCount, indexCount)`** →
   `{ vertexOffset, vertexCount, indexOffset, indexCount }`. Can fail if the pool is full — catch
   it and log, **never throw** into the render loop.
4. **Build `data`** as flat `Float32Array`s of length `vertexCount * components`, keyed by each
   `attr.field`, plus a `Uint32Array` of indices (the pool wants 32-bit). Bake your geometry in
   **world-space** (compute final positions yourself) — or upload local positions and let
   `upload({ ox, oz })` offset X/Z.
5. **`misc.upload(handle, data, { ox: 0, oz: 0 })`** — returns a promise.
6. **Borrow a live draw.** Each frame scan `renderQueue.draws` for a real draw whose
   `geometry.slab` is in `misc.slabs` (its pipeline is guaranteed to match the misc vertex
   format); take its `pipeline` + `groups`.
7. **Build a draw entry and re-push it every frame** (deduped by reference):

      const myDraw = {
        pipeline:     ref.pipeline,
        index:        ref.index,               // a key present in ref.groups
        geometry:     handle,
        instances:    null,
        groups:       ref.groups,
        shadowGroups: ref.shadowGroups ?? null,
        bounds:       [ minx, miny, minz, maxx, maxy, maxz ],   // AABB for culling
        shadowIndex:  ref.shadowIndex ?? 0,
        castsShadow:  false,
        shadowOnly:   false,
      };
      if (renderQueue && Array.isArray(renderQueue.draws) &&
          renderQueue.draws.indexOf(myDraw) === -1) renderQueue.draws.push(myDraw);

The geometry is depth-tested against the world, so it sits correctly in (and behind) the scene —
the whole reason to choose this over a DOM billboard.

### C. Textured geometry: your own pixels + material slot targeting (the `nameplates3d` recipe)

Geometry-only draws (`route-sign`) sample the *material's* textures. To carry **your own pixels**
(names, numbers) on scene geometry you need two more pieces, both engine-internal — neither is
`terra` API — and one genuinely tricky part (slot targeting). All of this was probed live
(`scene-probe` sandbox) and **confirmed working in-game 2026-11-11** (`nameplates3d` v0.2.9,
local sandbox — see AGENTS.md §6).

1. **Texture** — create through the engine's device *wrapper*, scope-owned so it's
   auto-destroyed with the feature:
       const TU  = GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST;
       const tex = eng.device.texture(featScope, { size: [w, h], format: 'rgba8unorm', usage: TU });
       const v   = tex.createView();
2. **Upload** — the wrapper's `writeTexture` is a game-chunk JS façade. The in-game winner is
   `eng.device.writeTexture(tex, data)` where `data` is a **typed array**
   (`ctx2d.getImageData(0, 0, w, h).data`). An object in that position errors *silently*, and
   the wrapper returns nothing — treat it as fire-and-forget, don't chain `.then()`.
3. **Bind our view everywhere the material samples** — build the bind group with the engine's
   *own* builder so it matches the expected `textures` layout by construction:
   `eng.textures.group([v, v, v, …])` (the builder pads any remaining slots with `white`;
   nameplates3d fills 32 slots with its view). Then swap it into the cloned draw's `groups`
   array at the slot the material actually samples (below).

**Finding that slot is the tricky part**, because in-engine bind groups **expose no
`.layout`** — the slot index can't be read directly. The only source of truth is the WebGPU
**validator**, and it is surprisingly readable:

- Binding a `textures` group into a slot that expects another layout (e.g. `material`)
  produces `…does not match layout [BindGroupLayout "material"]… at group index N` — it names
  **both** the expected layout and the offending slot. Parse exactly that.
- That error is delivered on the device's **`uncapturederror`** event — but on the **real
  GPUDevice, which in this engine lives at `eng.device.gpu`** (not the `eng.device` wrapper;
  a wrapper can't dispatch the event). Handler shape (spec): the `GPUValidationError` is
  **`ev.error`**, message at `ev.error.message`:
      const real  = eng.device.gpu;
      const prior = real.onuncapturederror;
      real.onuncapturederror = (ev) => {
        const msg = (ev.error && ev.error.message) || ev.message || '';
        /* /BindGroupLayout "([A-Za-z0-9_]+)"/ + /group index (\d+)/ →
           slotWant[N] = expected layout; our own layout name is on the same line */
        if (prior) prior(ev);                       // keep any pre-existing handler
      };
  Check-and-rearm per frame **only when the slot is empty** — never clobber a handler the
  engine installs later.
- **Slot policy** (deterministic, per frame): ① a slot *known to expect our layout* is the
  only safe pick; ② otherwise an *unflagged* slot is a blind try — it costs exactly one bad
  frame, but must not repeat; ③ **never** bind into a slot named for a *different* layout
  (`material` etc.) — prefer holding off (fall back to the reference groups) over an unflagged
  guess. An error naming a slot we are **not** bound to belongs to another scene draw: ignore
  it for targeting (don't pollute the oracle with foreign draws).
- A layout-mismatched bind group invalidates the **whole command buffer** for that frame —
  that's the one bad frame a blind try costs. The engine *survives* a single
  `Invalid CommandBuffer` (proven in-game), but a draw that stays mismatched spams errors and
  can stall the ticker — so the ladder has to **converge**, not flapping between two bad slots.
  In the confirmed run: `want=1:material` flagged slot 1, retarget hit slot 3, and the engine
  accepted it from then on.

**Texture look — alpha gotcha (two layers):** the borrowed pipeline **alpha-blends** the
drawn colour, and the alpha it uses is the *material's*, not just your texture's.

- **Layer 1 — your texture.** Anything with transparent margins renders as a translucent
  slab in the world (a 55%-alpha pill nameplate read as a floating dark rectangle). Fill
  the whole drawn area opaque (premultiplied-safe), or size the quad to the opaque part.
- **Layer 2 — the donor material itself.** Even a fully-opaque texture stayed translucent
  in-game (confirmed 2026-11-11): the reference draw you cloned carries the *guardrail-type
  material's own* semi-transparent blend, and you can't set its uniform from the addon
  (no `material.opacity` surface; the bind-group slots only hold engine-built groups).
  The practical cure is **accumulation**: push the *same* draw entry several times per
  frame (shallow copies that **share the `groups` object** — any per-frame slot retarget
  you apply to the primary then reaches every copy automatically; re-sync only the
  moving `bounds`). Under standard `SRC_ALPHA`/`ONE_MINUS_SRC_ALPHA` blending the
  effective alpha of N passes is `1-(1-a)^N` (a=0.55 → 3 passes ≈ 0.88; a=0.8 → 3 ≈ 0.99),
  which reads as solid. Cost: N× fragment fill of *your* quads — tiny for plates, don't
  do it for large geometry. `nameplates3d` exposes this as the `passes` setting (default 3).
- If you need the ground truth on the blend, tap `device.createRenderPipeline`
  (engine *wrapper* and real GPUDevice) before the scene builds its pipelines and read
  `desc.fragment.targets[0].blend` + the WGSL — that's the only exact source. The
  `scene-probe` sandbox / `nameplates3d` panel do a best-effort version of this.

### D. Reading the engine's live state (probe-first when in doubt)

Every name above is deploy-specific and hash-prone. When something "stops matching", the
`scene-probe` sandbox (live folder only, §6) dumps the engine's real surfaces — `engine.device`
wrapper vs `engine.device.gpu` (the true `GPUDevice`: `createTexture ✓` there), `engine.textures.group`,
`pools.misc.{slabs,format,allocate,upload}`, the `dev.writeTexture` source — into its in-page
panel. Read that before reading code.

> **Don't-break list when touching the render loop** (see AGENTS.md §9): `frame()` must **never
> throw** — the engine's frame ticker escalates any exception into a fatal “GPU can’t keep up”
> failure. Use `let`, not `const`, for anything you re-assign per frame (e.g. lazy element
> creation). And avoid heavy CSS — especially `backdrop-filter` — in in-page overlays; it broke the
> game's Vulkan/ANGLE stack.
