# Terra addons

An addon is a folder with a `manifest.json`. Terra scans, in order:
`$TERRA_ADDONS_DIR` (if set), otherwise `~/.local/share/dev.frazse.terra/addons/` and `./addons/`.
Folders starting with `_` or `.` are ignored. The folders are rescanned on every page load,
so edit an addon and press **F5** (or Ctrl+R) in the Terra window. No rebuild.

**Trust model:** addons run inside the biketerra.com page with the page's privileges (like
Tampermonkey userscripts), including the BLE commands. Only install addons you trust.

## Turning addons on and off
Click the **cog** next to Biketerra's theme toggle: every addon found in the folders is listed with a switch.
Changes apply immediately (the addon is started or cleaned up) and are remembered in
`~/.local/share/dev.frazse.terra/addon-data/.addons-state.json`. The green dot means it is running now.

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
