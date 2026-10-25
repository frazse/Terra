# Karoo Bridge — companion app (for the Hammerhead Karoo)

Turns a Hammerhead Karoo into a live, GPS-following bike computer for your **Biketerra**
ride, with real sensor data. Terra (on your computer) broadcasts the live ride over
Wi-Fi; this app (on the Karoo) receives it and does two things:

1. **Mock GPS** — injects every packet as a GPS fix, so the map, distance and
   elevation follow the ride in real time.
2. **Virtual sensors (Karoo extension)** — exposes the ride's *power, cadence, heart
   rate, speed and gears* as pairable sensors, so a Karoo activity records them like
   any Bluetooth sensor (the same mechanism KPower uses).

## How it works

```
Biketerra page (Terra)                    Hammerhead Karoo
─────────────────────                     ─────────────────────
biketerra-ride broadcast ─┐
karoo-bridge addon:       │
  normalize + throttle ───┼── UDP 41234 ──► KarooBridgeService:
  terra.udp.send(...)     │                  • mock GPS injection (map/distance)
realistic-gears:changed ──┘                  • Karoo extension "biketerra":
                                              one sensor pump per paired uid,
                                              pushing DataPoints at ~2 Hz
```

- UDP datagrams arrive at ~2–4 Hz carrying:
  `{ ts, lat, lng, alt, speed (m/s), bearing (deg), grade, power, cadence, hr, dist,
  gears: { fd, fdMax, fdTeeth, rd, rdMax, rdTeeth, ratio } }`
- `lat`/`lng` are decimal degrees (any FIT-semicircle encoding is converted on the Terra side).
- Both devices must be able to reach each other over Wi-Fi (same network, or the router
  must not isolate them).

## What gets exposed to the Karoo

The app is a **Karoo extension** (Hammerhead's `karoo-ext` 1.1.9 framework, vendored
in `lib/` — see `lib/NOTICE.md`). In the Karoo's sensor scanner it appears as the
**Biketerra** puzzle-piece category with these pairable sensors:

| Sensor | Data pushed | Notes |
| --- | --- | --- |
| Biketerra Power | watts | only while the source reports a sample |
| Biketerra Cadence | rpm | only while the source reports a sample |
| Biketerra Heart Rate | bpm | only while the source reports a sample |
| Biketerra Speed | m/s → km/h | `0` while standing still |
| Biketerra Gears | front & rear gear index, teeth, max | from the `realistic-gears` addon |

A value shown as `0` in the Terra packet also means "sample missing", so power/
cadence/HR are *not* pushed when they are `0` (the Karoo simply shows no data for
that sensor instead of a fake zero).

## Build

```
./gradlew assembleDebug
```

Output: `app/build/outputs/apk/debug/app-debug.apk`

## Install & enable mock location

```
adb install app/build/outputs/apk/debug/app-debug.apk
adb shell appops set dev.frazse.karoobridge android:mock_location allow
```

If you don't have `adb` set up: sideload the APK onto the Karoo, then manually enable
*Settings → Security/Mock location* (and allow the *Mock location provider* /
*Location* permissions when the system asks).

## Using it

1. Open the app on the Karoo — note the **Karoo Wi-Fi IP** shown at the top.
2. On your computer, in Terra's addon settings for **karoo-bridge**, set
   *Karoo Wi-Fi IP Address* to that exact IP (UDP port defaults to `41234`).
3. On the Karoo, press **Start listening** and keep the app/notification alive.
4. **Pair the sensors (once):** Karoo → *Sensors* → *Scan for sensors* → pick
   **Biketerra** → pair the sensors you want (Power, Cadence, Heart Rate, Speed,
   Gears). They persist in the Karoo's sensor list afterwards.
5. Start (or spectate) a ride on Biketerra in Terra. The app's *Datagrams* counter
   should climb at ~2–4/s, the Karoo should start moving with the ride, and the
   paired sensors should start reporting.

While the Karoo has sensors paired from the extension, its *Sensors* screen shows
connection status (searching → connected) for each — and the app's *Sensors paired*
line tells you which ones the Karoo currently holds.

## Notes

- **Keep the bridge running.** As long as the app's foreground service runs (or the
  Karoo has the extension bound), data flows — closing the app UI is fine. "Stop
  listening" removes the foreground state; if the Karoo still holds the sensors it
  keeps the service (and the data stream) alive until the ride ends.
- **Rescan after updating the app.** Known Karoo behaviour: virtual sensors from an
  updated extension sometimes need a re-scan before they connect again.
- The `realistic-gears` gear state (from the other Terra addon) rides along in the
  `gears` field, once the first shift event has been emitted.
- If the counter doesn't move: check both Wi-Fi IPs, that the Terra addon is enabled,
  and that no firewall on your computer blocks outbound UDP.
