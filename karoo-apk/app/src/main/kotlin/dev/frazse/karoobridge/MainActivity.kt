package dev.frazse.karoobridge

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.graphics.Typeface
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.ViewGroup
import android.widget.Button
import android.widget.CheckBox
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

/**
 * Control panel for the Karoo: shows the device Wi-Fi IP and the current telemetry
 * being injected as mock GPS, and toggles the UDP/mock-GPS listening service.
 */
class MainActivity : Activity() {

    private val handler = Handler(Looper.getMainLooper())
    private lateinit var summaryView: TextView
    private lateinit var liveView: TextView
    private lateinit var toggleButton: Button
    private lateinit var mockGpsButton: Button
    private lateinit var mockAutoCheck: CheckBox

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // If the service isn't running yet, the in-memory settings are just their
        // defaults — migrate any legacy (profile-name era) keys, then refresh
        // from disk so the UI shows the persisted choice.
        KarooBridgeService.migrate(this)
        val p = getSharedPreferences(KarooBridgeService.PREFS, Context.MODE_PRIVATE)
        KarooBridgeService.applySettings(
            this,
            p.getBoolean(KarooBridgeService.PREF_MOCK_GPS, true),
            p.getBoolean(KarooBridgeService.PREF_AUTO, true),
        )

        val pad = (resources.displayMetrics.density * 24).toInt()
        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad, pad, pad)
        }

        @Suppress("DEPRECATION")
        val apkVersion = packageManager.getPackageInfo(packageName, 0).versionName
        layout.addView(TextView(this).apply {
            text = "Karoo GPS Bridge  v$apkVersion"
            textSize = 28f
            setTypeface(typeface, Typeface.BOLD)
            maxLines = 3
        })
        layout.addView(TextView(this).apply {
            text = "Terra sends its live Biketerra ride over UDP (GPS, power, cadence, heart " +
                "rate, speed, gears). This app injects the GPS as a mock location and exposes " +
                "the rest to the Karoo as pairable virtual sensors."
            textSize = 14f
        })

        summaryView = TextView(this).apply {
            textSize = 15f
            val lp = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
            lp.topMargin = pad
            layoutParams = lp
        }
        layout.addView(summaryView)

        toggleButton = Button(this).apply {
            text = "Start listening"
            val lp = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
            lp.topMargin = pad
            layoutParams = lp
        }
        toggleButton.setOnClickListener {
            val intent = Intent(this, KarooBridgeService::class.java)
            if (KarooBridgeService.listening) {
                stopService(intent)
            } else {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    startForegroundService(intent)
                } else {
                    startService(intent)
                }
            }
            updateUi()
        }
        layout.addView(toggleButton)

        mockGpsButton = Button(this).apply {
            val lp = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
            lp.topMargin = pad
            layoutParams = lp
        }
        layout.addView(mockGpsButton)

        mockAutoCheck = CheckBox(this).apply {
            text = "AUTO: mock GPS only while the Terra game stream is live (fresh frames)"
            textSize = 14f
            setPadding(0, pad / 2, 0, 0)
        }
        layout.addView(mockAutoCheck)

        mockGpsButton.setOnClickListener {
            // The button flips the EFFECTIVE state, always predictably:
            //   ACTIVE -> OFF      (manual base off)
            //   STANDBY -> ACTIVE (manual takeover: auto off, base on)
            //   OFF    -> ACTIVE (manual base on)
            // Tapping while auto is idle forces manual mode instead of just
            // clearing the manual base — that used to read as "nothing happened".
            val nowActive = KarooBridgeService.mockGpsEffective
            KarooBridgeService.applySettings(
                this@MainActivity,
                mockGps = !nowActive,
                auto = if (nowActive) KarooBridgeService.mockAuto else false,
            )
            KarooBridgeService.instance?.syncProvider()
            updateUi()
        }
        mockAutoCheck.setOnClickListener {
            KarooBridgeService.applySettings(
                this@MainActivity,
                KarooBridgeService.mockGpsEnabled,
                !KarooBridgeService.mockAuto,
            )
            KarooBridgeService.instance?.syncProvider()
            updateUi()
        }

        liveView = TextView(this).apply {
            textSize = 15f
            typeface = Typeface.create(Typeface.MONOSPACE, Typeface.NORMAL)
            val lp = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
            lp.topMargin = pad
            layoutParams = lp
        }
        layout.addView(liveView)

        val scroll = ScrollView(this).apply {
            isFillViewport = true
            addView(layout)
        }
        setContentView(scroll)
    }

    private val ticker = object : Runnable {
        override fun run() {
            updateUi()
            handler.postDelayed(this, 500)
        }
    }

    override fun onResume() {
        super.onResume()
        handler.post(ticker)
    }

    override fun onPause() {
        super.onPause()
        handler.removeCallbacks(ticker)
    }

    /** This device's (the Karoo's) local Wi-Fi IP — must match Terra's `karoo_ip` setting. */
    private fun wifiIp(): String {
        val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
            ?: return "unknown"
        val ip = (wifi.connectionInfo.ipAddress).toLong() and 0xFFFFFFFFL
        if (ip == 0L) return "not on Wi-Fi?"
        return "${ip and 0xFF}.${(ip shr 8) and 0xFF}.${(ip shr 16) and 0xFF}.${(ip shr 24) and 0xFF}"
    }

    private fun updateUi() {
        val s = KarooBridgeService
        val eff = s.mockGpsEffective
        val standby = !eff && s.mockGpsEnabled && s.mockAuto
        toggleButton.text = if (s.listening) "Stop listening" else "Start listening"
        mockGpsButton.text = when {
            eff -> "MOCK GPS: ACTIVE (game location)  →  tap: real GNSS"
            standby -> "MOCK GPS: STANDBY — auto follows the game stream  →  tap: turn ON now"
            else -> "MOCK GPS: OFF (real GNSS)  →  tap: enable"
        }
        mockAutoCheck.isChecked = s.mockAuto

        summaryView.text = buildString {
            appendLine("Karoo Wi-Fi IP:  ").append(wifiIp())
            appendLine("UDP port:        ").append(s.UDP_PORT)
            appendLine("Service:         ").append(if (s.listening) "LISTENING" else "stopped")
            appendLine("Datagrams:       ").append(s.packetCount)
            appendLine("Sensors paired:  ").append(s.sensorsConnected())
            appendLine("Route for map:   ").append(if (s.routeReady) "received (${s.routeChars} chars)" else "not received yet")
            appendLine("Mock GPS:        ")
                .append(when {
                    eff -> "ACTIVE — the Karoo is receiving the game's GPS"
                    standby -> "STANDBY — auto engages while the Terra game stream is fresh"
                    else -> "OFF — the Karoo is using its real GNSS"
                })
            appendLine("Auto:            ")
                .append(if (s.mockAuto) "on — follows the game stream (releases ~2 min after it stops)"
                         else "off — mock on/off is manual")
            val at = s.lastFrameAtMs
            appendLine("Game stream:     ")
                .append(if (at > 0) ((System.currentTimeMillis() - at) / 1000L).toString() + "s since last frame"
                        else "no frames yet")
            appendLine("Elevation:       ").append(
                "if the Karoo's altitude differs from the game, set 'Elevation offset (m)' in the Terra addon = (game − Karoo)")
            s.activeProfileName?.let { name ->
                val kind = s.activeProfileIndoor?.let { if (it) "indoor (no GPS)" else "outdoor" } ?: "?"
                appendLine("Active profile:  ").append(name).append("  [").append(kind).append("]")
            } ?: appendLine("Active profile:  ").append("— (no profile event yet)")
            s.lastError?.let { append("Last error:      ").append(it).appendLine() }
        }
        val lat = s.lastLat
        val lng = s.lastLng
        liveView.text = buildString {
            append("last fix:  ")
            if (lat != null && lng != null) {
                append(String.format(java.util.Locale.US, "%.6f, %.6f", lat, lng))
                s.lastAlt?.let { append("    alt ").append(it.toInt()).append(" m") }
                s.lastSpeed?.let { append(String.format(java.util.Locale.US, "    %.1f m/s", it)) }
                s.lastBearing?.let { append("    hdg ").append(it.toInt()).append("°") }
            } else {
                append("—")
            }
            appendLine()
            append("power:     ")
            s.lastPower?.let { append(it).append(" W      ") } ?: append("—        ")
            append("cadence:   ")
            s.lastCadence?.let { append(it).append(" rpm   ") } ?: append("—      ")
            append("hr:        ")
            s.lastHr?.let { append(it).append(" bpm  ") } ?: append("—     ")
            appendLine()
            append("gears:     ").append(s.lastGears)
        }
    }
}
