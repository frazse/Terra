package dev.frazse.karoobridge

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.location.Location
import android.location.LocationManager
import android.location.provider.ProviderProperties
import android.os.Build
import android.util.Log
import io.hammerhead.karooext.KarooSystemService
import io.hammerhead.karooext.extension.KarooExtension
import io.hammerhead.karooext.internal.Emitter
import io.hammerhead.karooext.models.ActiveRideProfile
import io.hammerhead.karooext.models.BatteryStatus
import io.hammerhead.karooext.models.ConnectionStatus
import io.hammerhead.karooext.models.DataPoint
import io.hammerhead.karooext.models.DataType
import io.hammerhead.karooext.models.Device
import io.hammerhead.karooext.models.DeviceEvent
import io.hammerhead.karooext.models.ManufacturerInfo
import io.hammerhead.karooext.models.MapEffect
import io.hammerhead.karooext.models.OnBatteryStatus
import io.hammerhead.karooext.models.OnConnectionStatus
import io.hammerhead.karooext.models.OnDataPoint
import io.hammerhead.karooext.models.OnManufacturerInfo
import io.hammerhead.karooext.models.ShowPolyline
import io.hammerhead.karooext.models.ShowSymbols
import io.hammerhead.karooext.models.Symbol
import org.json.JSONArray
import org.json.JSONObject
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.util.concurrent.ConcurrentHashMap

/**
 * The bridge between Terra (Biketerra ride, over UDP) and the Hammerhead Karoo.
 *
 * One service, two roles:
 *  1. Foreground service started from the app UI — listens on [UDP_PORT] and
 *     injects each datagram as a mock GPS fix (map/distance/elevation follow).
 *  2. Karoo *extension* (see vendored `io.hammerhead.karooext` in :lib) — the
 *     Karoo system binds this service, calls [startScan]/[connectDevice], and
 *     we push the real power / cadence / HR / speed / gear values out of each
 *     UDP frame as sensor data points.
 *
 * Because the sensor pumps read the latest decoded frame from this process,
 * the ride's data keeps flowing for a paired sensor as long as the Karoo has
 * the sensor connected, even if the app UI is closed.
 */
class KarooBridgeService : KarooExtension("biketerra", "1.0.0") {

    companion object {
        const val UDP_PORT = 41234
        private const val TAG = "KarooBridge"
        private const val NOTIF_CHANNEL = "karoobridge"
        private const val NOTIF_ID = 1041

        // Sensor uids, one per pairable "sensor" in the Karoo scan.
        const val UID_POWER = "biketerra-power"
        const val UID_CADENCE = "biketerra-cadence"
        const val UID_HR = "biketerra-hr"
        const val UID_SPEED = "biketerra-speed"
        const val UID_GEAR = "biketerra-gears"

        private val SENSOR_NAMES = mapOf(
            UID_POWER to "power",
            UID_CADENCE to "cadence",
            UID_HR to "heart rate",
            UID_SPEED to "speed",
            UID_GEAR to "gears",
        )

        const val ROUTE_POLY_ID = "biketerra-route"
        const val ROUTE_COLOR = 0xFFFF8A00.toInt() // amber, reads well on the dark map
        const val ROUTE_CHUNK_WINDOW_MS = 4000L

        // Kill-switch for pushing the route into the Karoo's map (startMap path).
        // v3 regression: with mapLayer enabled, opening a ride made io.hammerhead.rideapp
        // crash in its effect handler (StringIndexOutOfBounds while decoding a short
        // string — our route polyline is the prime suspect). Route assembly/logging stays
        // on so we can read assembled lengths from logcat when we re-test the map.
        const val ROUTE_TO_MAP = false

        // Live state for the UI: written from the UDP thread, polled by the Activity.
        @Volatile var listening = false
            private set
        @Volatile var appStarted = false            // started from the UI (FGS requested)
        @Volatile var packetCount = 0L
            private set
        @Volatile var lastError: String? = null
        @Volatile var lastLat: Double? = null
        @Volatile var lastLng: Double? = null
        @Volatile var lastAlt: Double? = null
        @Volatile var lastSpeed: Double? = null
        @Volatile var lastBearing: Double? = null
        @Volatile var lastPower: Int? = null
        @Volatile var lastCadence: Int? = null
        @Volatile var lastHr: Int? = null
        @Volatile var lastGears = "—"

        // Route status (set when the chunked route polyline has been received).
        @Volatile var routeReady = false
            private set
        @Volatile var routeChars = 0
            private set

        val activeSensorUids = ConcurrentHashMap.newKeySet<String>()

        fun sensorsConnected(): String = activeSensorUids
            .joinToString(", ") { SENSOR_NAMES[it] ?: it }
            .ifEmpty { "none" }

        // ---- Mock-GPS (test-location) control --------------------------------
        // The test provider is registered against `gps` (GPS_PROVIDER), so while
        // engaged it shadows the Karoo's real GNSS.
        //   base = the mock-GPS master switch (manual, default ON)
        //   auto = follow the game stream (default ON): the mock engages only
        //          while Terra's ride frames are fresh (last frame within
        //          [STREAM_GRACE_MS]); when the stream goes quiet — ride over,
        //          page closed — the Karoo falls back to its real GNSS on its
        //          own. This replaces the v3.2 (indoor-flag) and v3.3
        //          (profile-name) auto modes, which never engaged reliably.
        const val PREFS = "karoobridge"
        const val PREF_MOCK_GPS = "mock_gps"   // default true
        const val PREF_AUTO = "mock_gps_auto"  // default true (stream-follow)
        // Legacy keys from the profile-name era (v3.3), removed on startup:
        const val PREF_AUTO_INDOOR = "mock_gps_auto_indoor"
        const val PREF_PROFILE_NAME = "mock_gps_profile_name"
        const val PREF_KNOWN_PROFILES = "mock_gps_known_profiles"
        const val PREF_SELF_TEST_REMOVED = "self_test" // legacy key of the (removed) self-test

        // Stream-freshness window for auto mode: the mock stays engaged this
        // long after the last frame, so a ride-over keeps the game location
        // for a grace period before real GNSS takes over.
        const val STREAM_GRACE_MS = 120_000L

        @Volatile var mockGpsEnabled = true
            private set
        @Volatile var mockAuto = true
            private set

        // Last telemetry frame seen (ms epoch; 0 = never). Auto mode watches this.
        @Volatile var lastFrameAtMs = 0L
            private set

        // Filled from the Karoo's `ActiveRideProfile` system event (informational only).
        @Volatile var activeProfileName: String? = null
        @Volatile var activeProfileIndoor: Boolean? = null

        /** True while Terra's stream counts as fresh (a frame within [STREAM_GRACE_MS]). */
        val streamFresh: Boolean
            get() {
                val at = lastFrameAtMs
                return at > 0 && System.currentTimeMillis() - at < STREAM_GRACE_MS
            }

        /**
         * Effective mock-GPS state: the base switch, and — while auto
         * (stream-follow) is on — the game stream must be fresh. Exposed on
         * the companion (static) so the UI can display it directly.
         */
        val mockGpsEffective: Boolean
            get() = mockGpsEnabled && (!mockAuto || streamFresh)

        // Process-wide handle so the Activity can reach the running service
        // (same process) without any reference ceremony.
        @Volatile var instance: KarooBridgeService? = null
            private set

        /** Persist the UI's mock-GPS choice and refresh the in-memory state. */
        fun applySettings(ctx: Context, mockGps: Boolean, auto: Boolean) {
            ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .edit()
                .putBoolean(PREF_MOCK_GPS, mockGps)
                .putBoolean(PREF_AUTO, auto)
                .apply()
            changed(mockGps, auto)
        }

        private fun changed(mockGps: Boolean, auto: Boolean) {
            if (mockGpsEnabled != mockGps || mockAuto != auto) {
                Log.i(TAG, "settings: mockGps=$mockGps auto=$auto")
                mockGpsEnabled = mockGps
                mockAuto = auto
            }
        }

        /**
         * One-shot migration out of the profile-name era (v3.3) settings;
         * idempotent — it only acts while legacy keys still exist. The v4
         * mode is "follow the game stream", so a user who had the old auto
         * mode in use ends up with auto (stream-follow) ON. Never clobbers a
         * deliberately chosen value once the legacy keys are gone.
         */
        fun migrate(ctx: Context) {
            val prefs = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val legacy = prefs.contains(PREF_AUTO_INDOOR) ||
                prefs.contains(PREF_PROFILE_NAME) || prefs.contains(PREF_KNOWN_PROFILES)
            if (!legacy && !prefs.contains(PREF_SELF_TEST_REMOVED)) return
            prefs.edit()
                .remove(PREF_AUTO_INDOOR)
                .remove(PREF_PROFILE_NAME)
                .remove(PREF_KNOWN_PROFILES)
                .remove(PREF_SELF_TEST_REMOVED)
                .apply()
            if (legacy) {
                prefs.edit().putBoolean(PREF_AUTO, true).apply()
                Log.i(TAG, "settings migrated: profile-name auto mode -> stream-follow auto (on)")
            }
        }
    }

    /** One decoded UDP frame (immutable snapshot for the pump threads and UI).
     *  `null` = the field was missing from the datagram (don't emit it);
     *  `0.0` = a genuine zero reading (coasting / standing still) — emit it. */
    data class Frame(
        val lat: Double?,
        val lng: Double?,
        val alt: Double?,
        val speed: Double?,      // m/s
        val bearing: Double?,    // deg
        val grade: Double?,
        val dist: Double?,
        val power: Double?,      // W
        val cadence: Double?,    // rpm
        val hr: Double?,         // bpm
        val gears: Gears?,
    ) {
        data class Gears(
            val fd: Int, val fdMax: Int, val fdTeeth: Int,
            val rd: Int, val rdMax: Int, val rdTeeth: Int,
            val ratio: Double,
        )
    }

    @Volatile private var latest: Frame? = null
    private var socketRef: DatagramSocket? = null
    private val pumps = ConcurrentHashMap<String, Thread>()

    // -- Mock-GPS provider state ------------------------------------------------
    @Volatile private var providerEnabled: Boolean? = null // last confirmed state
    private var karooSystem: KarooSystemService? = null
    private var autoTicker: java.util.concurrent.ScheduledExecutorService? = null

    /**
     * Effective mock-GPS state (see [mockGpsEffective]).
     */
    val mockGpsActive: Boolean
        get() = mockGpsEffective

    /**
     * Reconcile the platform test-location provider with [mockGpsActive]:
     * enabled, it shadows the Karoo's real `gps` provider; disabled, real
     * GNSS resumes. Idempotent — only touches the LocationManager on a real
     * change.
     *
     * Requires the `android:mock_location` appop (see README: `adb shell appops set ...`).
     */
    fun syncProvider() {
        val want = mockGpsActive
        if (providerEnabled == want) return
        val lm = getSystemService(LOCATION_SERVICE) as? LocationManager ?: return
        try {
            if (want) {
                val props = ProviderProperties.Builder()
                    .setHasSatelliteRequirement(true)
                    .setHasAltitudeSupport(true)
                    .setHasBearingSupport(true)
                    .setHasSpeedSupport(true)
                    .setPowerUsage(ProviderProperties.POWER_USAGE_LOW)
                    .setAccuracy(ProviderProperties.ACCURACY_FINE)
                    .build()
                try {
                    lm.addTestProvider(LocationManager.GPS_PROVIDER, props)
                } catch (_: Exception) {
                    // Already registered (e.g. service restart) — that's fine.
                }
                lm.setTestProviderEnabled(LocationManager.GPS_PROVIDER, true)
            } else {
                runCatching { lm.setTestProviderEnabled(LocationManager.GPS_PROVIDER, false) }
                // Disabling alone is not enough: once a test provider has been
                // registered against `gps`, the Karoo app can end up with its
                // real `gps` provider dead ("gps disabled" on the next outdoor
                // ride). Remove the test provider entirely so the real GNSS
                // provider is the only one in place again.
                runCatching { lm.removeTestProvider(LocationManager.GPS_PROVIDER) }
            }
            providerEnabled = want
            Log.i(TAG, "mock-GPS provider -> " +
                if (want) "ON (mock shadows real GNSS)" else "OFF (real GNSS resumes)")
        } catch (e: Exception) {
            Log.w(TAG, "mock-GPS provider sync failed: ${e.message}")
        }
    }

    /**
     * Track the Karoo's active ride profile (name + `indoor` flag) so the UI
     * can show it. Informational only — v4 auto mode follows the GAME STREAM
     * (freshness of the UDP frames), not the Karoo's profile. Needs ext lib
     * ≥1.1.5; the vendored 1.1.9 ships it. A failed bind is non-fatal.
     */
    private fun startProfileEvents() {
        try {
            val system = KarooSystemService(applicationContext)
            karooSystem = system
            system.addConsumer<ActiveRideProfile>(
                ActiveRideProfile.Params,
                onEvent = { event ->
                    val profile = event.profile
                    Log.i(TAG, "active ride profile: ${profile.name} (indoor=${profile.indoor})")
                    activeProfileName = profile.name
                    activeProfileIndoor = profile.indoor
                },
            )
            system.connect()
        } catch (e: Exception) {
            Log.w(TAG, "profile tracking unavailable: ${e.message}")
        }
    }

    /**
     * Auto (stream-follow) mode is time-driven: the mock must engage when
     * Terra starts broadcasting and release ~[STREAM_GRACE_MS] after the
     * stream goes quiet. A slow ticker reconciles the test provider —
     * [syncProvider] is idempotent, so steady-state ticks are no-ops.
     */
    private fun startAutoTicker() {
        val t = java.util.concurrent.Executors.newSingleThreadScheduledExecutor { r ->
            Thread(r, "karoobridge-auto").apply { isDaemon = true }
        }
        autoTicker = t
        t.scheduleAtFixedRate(
            { runCatching { syncProvider() }.onFailure { e -> Log.w(TAG, "auto ticker: ${e.message}") } },
            5L, 5L, java.util.concurrent.TimeUnit.SECONDS,
        )
    }

    // ------------------------------------------------------------------ lifecycle

    override fun onCreate() {
        super.onCreate()
        @Suppress("DEPRECATION")
        val apk = packageManager.getPackageInfo(packageName, 0)
        Log.i(TAG, "service created (extension=$extension v$version, apk=${apk.versionName} code=${apk.versionCode})")
        instance = this
        createNotificationChannel()
        migrate(applicationContext) // one-shot: profile-name-era keys -> stream-follow
        val p = getSharedPreferences(PREFS, MODE_PRIVATE)
        applySettings(
            applicationContext,
            p.getBoolean(PREF_MOCK_GPS, true),
            p.getBoolean(PREF_AUTO, true), // v4 default: follow the game stream
        )
        syncProvider() // enable the mock provider only if mock GPS is active
        startAutoTicker() // engage/release the mock as the stream goes fresh/stale
        startProfileEvents()
        startListener()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        appStarted = true
        try {
            startForeground(NOTIF_ID, buildNotification())
        } catch (e: Exception) {
            // e.g. background-FGS start restriction; the service still works as a
            // bound Karoo extension, only the notification/FGS priority is lost.
            Log.w(TAG, "startForeground failed: ${e.message}")
        }
        return START_STICKY
    }

    override fun onDestroy() {
        Log.i(TAG, "service destroyed")
        autoTicker?.shutdownNow()
        autoTicker = null
        pumps.values.forEach { it.interrupt() }
        stopListener()
        runCatching { karooSystem?.disconnect() }
        karooSystem = null
        if (instance === this) instance = null
        // Give the Karoo back its real GNSS: disable AND remove the test
        // provider (leaving it registered-but-disabled can keep the Karoo's
        // real `gps` provider dead after a restart). A following onCreate
        // re-syncs from the persisted settings.
        runCatching {
            val lm = getSystemService(LOCATION_SERVICE) as? LocationManager
            lm?.setTestProviderEnabled(LocationManager.GPS_PROVIDER, false)
            lm?.removeTestProvider(LocationManager.GPS_PROVIDER)
            Log.i(TAG, "mock-GPS provider -> removed (real GNSS resumes)")
        }
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    stopForeground(STOP_FOREGROUND_REMOVE)
                } else {
                    @Suppress("DEPRECATION") stopForeground(true)
                }
            } else {
                @Suppress("DEPRECATION") stopForeground(true)
            }
        } catch (_: Exception) {
        }
        super.onDestroy()
    }

    // --------------------------------------------------------------- scan & connect

    private fun deviceFor(uid: String): Device = when (uid) {
        UID_POWER -> Device(extension, uid, listOf(DataType.Source.POWER), "Biketerra Power")
        UID_CADENCE -> Device(
            extension, uid,
            // Declare BOTH spellings of the cadence type: the "raw sensor" source id
            // and the Karoo display type, so pairing/recording matches whichever
            // the Karoo routes on. (Power/HR/Gears need only one spelling because
            // both groups carry the identical string for those.)
            listOf(DataType.Source.CADENCE, DataType.Type.CADENCE),
            "Biketerra Cadence",
        )
        UID_HR -> Device(extension, uid, listOf(DataType.Source.HEART_RATE), "Biketerra Heart Rate")
        UID_SPEED -> Device(
            extension, uid,
            // Both spellings, same reasoning as cadence — see above.
            listOf(DataType.Source.SPEED, DataType.Type.SPEED),
            "Biketerra Speed",
        )
        UID_GEAR -> Device(
            extension, uid,
            listOf(DataType.Source.SHIFTING_FRONT_GEAR, DataType.Source.SHIFTING_REAR_GEAR),
            "Biketerra Gears",
        )
        else -> Device(extension, uid, emptyList(), "Biketerra")
    }

    // ------------------------------------------------------------------ map layer

    // Route polyline state. The chunk fields are only ever touched by the single
    // UDP listener thread (datagrams are processed sequentially); the published
    // results are `@Volatile` because they may be read from the main thread.
    @Volatile private var routePolyline: String? = null
    @Volatile private var routeStart: DoubleArray? = null
    @Volatile private var routeEnd: DoubleArray? = null
    private var slotTotal = 0
    private val slotParts = HashMap<Int, String>()
    private var slotStartAt = 0L

    @Volatile private var mapEmitter: Emitter<MapEffect>? = null

    /**
     * The Karoo calls this to offer the extension a slice of its map. While this
     * session is open we render the Biketerra route (if one was received) as a
     * polyline plus start/end markers. Kept live: if the route arrives *after*
     * the session opened, [pushRouteToMap] sends it into the open emitter.
     */
    override fun startMap(emitter: Emitter<MapEffect>) {
        Log.i(TAG, "startMap (route ${if (routeReady) routeChars else "?"} chars, to-map=$ROUTE_TO_MAP)")
        if (!ROUTE_TO_MAP) return // v3 regression in the Karoo's effect handler — stay off
        mapEmitter = emitter
        emitter.setCancellable { if (mapEmitter === emitter) mapEmitter = null }
        pushRouteToMap()
    }

    private fun pushRouteToMap() {
        val em = mapEmitter ?: return
        val poly = routePolyline ?: return
        runCatching {
            em.onNext(ShowPolyline(ROUTE_POLY_ID, poly, ROUTE_COLOR, 5))
            val syms = mutableListOf<Symbol>()
            routeStart?.let { syms.add(Symbol.POI(id = "bt-start", lat = it[0], lng = it[1], type = Symbol.POI.Types.CONTROL)) }
            routeEnd?.let { syms.add(Symbol.POI(id = "bt-end", lat = it[0], lng = it[1], type = Symbol.POI.Types.CONTROL)) }
            if (syms.isNotEmpty()) em.onNext(ShowSymbols(syms))
        }.onFailure { Log.w(TAG, "pushRouteToMap: ${it.message}") }
    }

    /** One datagram of a chunked route polyline (`{route:{seq,total,poly,…}}`). */
    private fun onRouteChunk(o: JSONObject) {
        val seq = o.optInt("seq", -1)
        val total = o.optInt("total", -1)
        val poly = o.optString("poly")
        if (total !in 1..10_000 || seq !in 0 until total || poly.isEmpty()) {
            Log.w(TAG, "route chunk dropped: seq=$seq total=$total poly=${poly.length}")
            return
        }
        val now = System.currentTimeMillis()
        if (seq == 0 || slotTotal != total || now - slotStartAt > ROUTE_CHUNK_WINDOW_MS) {
            slotTotal = total
            slotParts.clear()
            slotStartAt = now
        }
        slotParts[seq] = poly
        if (seq == 0) {
            routeStart = o.optJSONArray("start")?.let(::doublePair)
            routeEnd = o.optJSONArray("end")?.let(::doublePair)
        }
        if (slotParts.size >= total) {
            val line = StringBuilder(total * 2)
            for (i in 0 until total) line.append(slotParts[i])
            routePolyline = line.toString()
            slotParts.clear()
            slotTotal = 0
            routeReady = true
            routeChars = line.length
            lastError = null
            Log.i(TAG, "route assembled: ${line.length} chars (${total} chunks)")
            pushRouteToMap()
        }
    }

    private fun doublePair(a: JSONArray): DoubleArray? {
        if (a.length() < 2) return null
        val lat = a.optDouble(0, Double.NaN)
        val lng = a.optDouble(1, Double.NaN)
        return if (lat.isNaN() || lng.isNaN()) null else doubleArrayOf(lat, lng)
    }

    override fun startScan(emitter: Emitter<Device>) {
        Log.i(TAG, "startScan")
        val uids = listOf(UID_POWER, UID_CADENCE, UID_HR, UID_SPEED, UID_GEAR)
        for (uid in uids) {
            emitter.onNext(deviceFor(uid))
        }
    }

    override fun connectDevice(uid: String, emitter: Emitter<DeviceEvent>) {
        Log.i(TAG, "connectDevice $uid")
        pumps[uid]?.interrupt() // a re-pair replaces any in-flight pump for this uid
        val pump = Thread({ pumpLoop(uid, emitter) }, "karoobridge-pump-$uid").apply { isDaemon = true }
        pumps[uid] = pump
        emitter.setCancellable {
            pump.interrupt()
            if (pumps.remove(uid, pump)) activeSensorUids.remove(uid)
        }
        activeSensorUids.add(uid)
        pump.start()
    }

    /**
     * Push data points for one paired sensor until the system disconnects us.
     *
     * Emits [ConnectionStatus.SEARCHING] until a fresh frame is flowing, then
     * CONNECTED + battery + manufacturer info, then ~2Hz data points.
     * If frames go stale (> [STALE_MS]) reports DISCONNECTED and re-connects
     * when fresh data arrives again.
     */
    private fun pumpLoop(uid: String, emitter: Emitter<DeviceEvent>) {
        val staleMs = 4000L
        val minIntervalMs = 500L // ~2Hz; Karoo expects ~1Hz
        var connected = false
        var lastPush = 0L
        try {
            emitter.onNext(OnConnectionStatus(ConnectionStatus.SEARCHING))
            while (true) {
                Thread.sleep(250L)
                val frame = latest
                if (frame == null) continue
                val fresh = System.currentTimeMillis() - lastFrameAtMs <= staleMs
                if (!fresh) {
                    if (connected) {
                        connected = false
                        Log.i(TAG, "pump $uid: data stale, reporting DISCONNECTED")
                        emitter.onNext(OnConnectionStatus(ConnectionStatus.DISCONNECTED))
                    }
                    continue
                }
                if (!connected) {
                    connected = true
                    Log.i(TAG, "pump $uid: CONNECTED")
                    emitter.onNext(OnConnectionStatus(ConnectionStatus.CONNECTED))
                    emitter.onNext(OnBatteryStatus(BatteryStatus.GOOD))
                    emitter.onNext(
                        OnManufacturerInfo(
                            ManufacturerInfo("Terra", "BKT-0001", "BKT-${uid.removePrefix("biketerra-").uppercase()}")
                        )
                    )
                }
                val now = System.currentTimeMillis()
                if (now - lastPush < minIntervalMs) continue
                lastPush = now
                val points = dataPointsFor(uid, frame)
                for (dp in points) {
                    emitter.onNext(OnDataPoint(dp))
                }
            }
        } catch (e: InterruptedException) {
            Log.d(TAG, "pump $uid: cancelled")
        } catch (e: Exception) {
            // Remote end of the AIDL connection is gone (ride over / unparked sensor).
            Log.w(TAG, "pump $uid: ended: ${e.message}")
        }
    }

    // null = field missing from the frame → no data point at all;
    // 0.0 = genuine reading (coasting/standing still) → emit it as a real datum.
    private fun dataPointsFor(uid: String, frame: Frame): List<DataPoint> = when (uid) {
        UID_POWER -> frame.power?.let {
            listOf(DataPoint(DataType.Type.POWER, mapOf(DataType.Field.POWER to it), uid))
        } ?: emptyList()
        UID_CADENCE -> frame.cadence?.let {
            // POINT TYPE = declared source id (TYPE_CAD_CADENCE_ID). Previously we
            // sent the display id (TYPE_CADENCE_ID) while the sensor declared the
            // source id — and the Karoo never recorded it (power/HR work because
            // both spellings are the identical string for them).
            listOf(DataPoint(DataType.Source.CADENCE, mapOf(DataType.Field.CADENCE to it), uid))
        } ?: emptyList()
        UID_HR -> frame.hr?.let {
            listOf(DataPoint(DataType.Type.HEART_RATE, mapOf(DataType.Field.HEART_RATE to it), uid))
        } ?: emptyList()
        UID_SPEED -> frame.speed?.let {
            // Same fix as cadence: point type = declared source id.
            listOf(DataPoint(DataType.Source.SPEED, mapOf(DataType.Field.SPEED to it), uid))
        } ?: emptyList()
        UID_GEAR -> {
            val g = frame.gears ?: return emptyList()
            buildList {
                if (g.fd in 1..g.fdMax) {
                    add(
                        DataPoint(
                            DataType.Type.SHIFTING_FRONT_GEAR,
                            mapOf(
                                DataType.Field.SHIFTING_FRONT_GEAR to g.fd.toDouble(),
                                DataType.Field.SHIFTING_FRONT_GEAR_TEETH to g.fdTeeth.toDouble(),
                                DataType.Field.SHIFTING_FRONT_GEAR_MAX to g.fdMax.toDouble(),
                            ),
                            uid,
                        )
                    )
                }
                if (g.rd in 1..g.rdMax) {
                    add(
                        DataPoint(
                            DataType.Type.SHIFTING_REAR_GEAR,
                            mapOf(
                                DataType.Field.SHIFTING_REAR_GEAR to g.rd.toDouble(),
                                DataType.Field.SHIFTING_REAR_GEAR_TEETH to g.rdTeeth.toDouble(),
                                DataType.Field.SHIFTING_REAR_GEAR_MAX to g.rdMax.toDouble(),
                            ),
                            uid,
                        )
                    )
                }
            }
        }
        else -> emptyList()
    }

    // -------------------------------------------------------------------- UDP part

    private fun startListener() {
        if (listening) return
        listening = true
        Thread({ listenLoop() }, "karoobridge-udp").start()
    }

    private fun stopListener() {
        listening = false
        runCatching { socketRef?.close() }
        socketRef = null
    }

    private fun listenLoop() {
        var sock: DatagramSocket? = null
        try {
            sock = DatagramSocket(UDP_PORT)
            socketRef = sock
            Log.i(TAG, "UDP listening on :$UDP_PORT")
            val buf = ByteArray(4096)
            while (listening) {
                val packet = DatagramPacket(buf, buf.size)
                sock.receive(packet) // blocking
                val text = String(buf, 0, packet.length, Charsets.UTF_8).trim { it <= ' ' }
                if (text.isEmpty()) continue
                packetCount += 1
                onFrame(text)
            }
        } catch (e: Exception) {
            if (listening) {
                lastError = "UDP: ${e.message ?: e.javaClass.simpleName}"
                Log.w(TAG, "UDP loop died: ${e.message}")
            }
        } finally {
            runCatching { sock?.close() }
            socketRef = null
        }
    }

    private fun onFrame(text: String) {
        try {
            val j = JSONObject(text)
            j.optJSONObject("route")?.let { onRouteChunk(it); return }
            val gears = j.optJSONObject("gears")?.let { parseGears(it) }
            val frame = Frame(
                lat = dNum(j, "lat"),
                lng = dNum(j, "lng"),
                alt = dNum(j, "alt"),
                speed = dNum(j, "speed"),
                bearing = dNum(j, "bearing"),
                grade = dNum(j, "grade"),
                dist = dNum(j, "dist"),
                power = dNum(j, "power"),
                cadence = dNum(j, "cadence"),
                hr = dNum(j, "hr"),
                gears = gears,
            )
            latest = frame
            lastFrameAtMs = System.currentTimeMillis()
            updateUiState(frame)
            injectGps(frame)
        } catch (e: Exception) {
            lastError = "parse: ${e.message ?: e.javaClass.simpleName}"
        }
    }

    private fun parseGears(o: JSONObject): Frame.Gears {
        // gears arrives as { fd, fdMax, fdTeeth, rd, rdMax, rdTeeth, ratio } —
        // tolerate the longer `front*`/`rear*` spellings too.
        val fd = firstPositive(o, "fd", "front") ?: 0
        val rd = firstPositive(o, "rd", "rear") ?: 0
        return Frame.Gears(
            fd = fd,
            fdMax = firstPositive(o, "fdMax", "frontMax") ?: fd,
            fdTeeth = firstPositive(o, "fdTeeth", "frontTeeth") ?: 0,
            rd = rd,
            rdMax = firstPositive(o, "rdMax", "rearMax") ?: rd,
            rdTeeth = firstPositive(o, "rdTeeth", "rearTeeth") ?: 0,
            ratio = o.optDouble("ratio", 0.0),
        )
    }

    private fun firstPositive(o: JSONObject, vararg keys: String): Int? =
        keys.firstNotNullOfOrNull { o.optInt(it).takeIf { v -> v > 0 } }

    /** `null` when the key is absent or JSON `null`; a real number otherwise (incl. 0). */
    private fun dNum(j: JSONObject, key: String): Double? =
        if (j.has(key) && !j.isNull(key)) j.getDouble(key).toDouble() else null

    private fun updateUiState(frame: Frame) {
        lastLat = frame.lat
        lastLng = frame.lng
        lastAlt = frame.alt
        lastSpeed = frame.speed
        lastBearing = frame.bearing
        lastPower = frame.power?.toInt()
        lastCadence = frame.cadence?.toInt()
        lastHr = frame.hr?.toInt()
        lastGears = frame.gears?.let {
            "${it.fd}/${it.fdMax}-${it.rd}/${it.rdMax} (${it.fdTeeth}x${it.rdTeeth}T)"
        } ?: "—"
        lastError = null
    }

    private fun injectGps(frame: Frame) {
        if (!mockGpsActive) return // mock GPS disabled — the Karoo's real GNSS is in use
        if (providerEnabled != mockGpsActive) syncProvider() // self-heal a mid-run toggle
        val lat = frame.lat
        val lng = frame.lng
        if (lat == null || lng == null) return  // no GPS fix in this frame — skip
        val lm = getSystemService(LOCATION_SERVICE) as? LocationManager ?: return
        val loc = Location(LocationManager.GPS_PROVIDER).apply {
            setLatitude(lat)
            setLongitude(lng)
            frame.alt?.let {
                setAltitude(it)
                // Altitude trust signal. The horizontal position already carries
                // `accuracy`, so the Karoo trusts lat/lng — but with no vertical
                // accuracy the fix's altitude reads as invalid and the ride engine
                // falls back to the (stationary) barometer, latching the first
                // elevation for the whole ride. Mirror `accuracy` here.
                setVerticalAccuracyMeters(2.0f)
            }
            frame.speed?.let { if (it in 0.0..200.0) speed = it.toFloat() }  // 0 m/s is a real stop
            frame.bearing?.let { if (it in 0.0..360.0) bearing = (it % 360.0).toFloat() }
            accuracy = 1.5f
            time = System.currentTimeMillis()
            elapsedRealtimeNanos = android.os.SystemClock.elapsedRealtimeNanos()
        }
        runCatching { lm.setTestProviderLocation(LocationManager.GPS_PROVIDER, loc) }
            .onFailure {
                lastError = "injection: ${it.message ?: it.javaClass.simpleName}"
            }
    }

    // ------------------------------------------------------------------- notification

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val nm = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
            nm.createNotificationChannel(
                NotificationChannel(NOTIF_CHANNEL, "Biketerra bridge", NotificationManager.IMPORTANCE_LOW)
            )
        }
    }

    private fun buildNotification(): Notification {
        val piFlags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        } else {
            PendingIntent.FLAG_UPDATE_CURRENT
        }
        val openIntent = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java), piFlags
        )
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O)
            Notification.Builder(this, NOTIF_CHANNEL)
        else @Suppress("DEPRECATION")
            Notification.Builder(this)
        builder.setContentTitle("Biketerra bridge")
            .setContentText("Streaming ${sensorsConnected()} sensor(s) to the Karoo")
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setOngoing(true)
            .setContentIntent(openIntent)
        return builder.build()
    }
}
