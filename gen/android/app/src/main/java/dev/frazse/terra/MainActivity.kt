package dev.frazse.terra

import android.Manifest
import android.app.ActivityManager
import android.app.PictureInPictureParams
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Point
import android.graphics.Rect
import android.hardware.display.DisplayManager
import android.net.Uri
import android.net.http.SslError
import android.os.BatteryManager
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.os.Message
import android.os.PowerManager
import android.os.Process
import android.os.SystemClock
import android.provider.DocumentsContract
import android.provider.Settings
import android.util.DisplayMetrics
import android.util.Rational
import android.view.Display
import android.view.KeyEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.ClientCertRequest
import android.webkit.ConsoleMessage
import android.webkit.CookieManager
import android.webkit.GeolocationPermissions
import android.webkit.HttpAuthHandler
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.PermissionRequest
import android.webkit.SslErrorHandler
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.Keep
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import java.io.ByteArrayInputStream
import java.io.File
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URI
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class MainActivity : TauriActivity() {
  companion object {
    private var instance: MainActivity? = null
    private var dataDirResult: String = "pending" // "pending" | "cancelled" | "success:<path>" | "error:<msg>"
    private const val PREFS_NAME = "terra_prefs"
    private const val PREF_PIP_ENABLED = "pip_enabled"
    private const val PREF_PIP_AUTO_ENTER = "pip_auto_enter"
    private const val PREF_PIP_HUD_MODE = "pip_hud_mode"
    private const val PREF_PIP_SHOW_BUTTON = "pip_show_button"
    private const val PREF_DUAL_ENABLED = "dual_enabled"
    private const val PREF_DUAL_MODE = "dual_mode"
    private const val PREF_DUAL_KEEP_NAMEPLATES = "dual_keep_nameplates"

    @Keep
    @JvmStatic
    fun setFullscreenJni(enable: Boolean) {
      instance?.runOnUiThread {
        instance?.toggleFullscreen(enable)
      }
    }

    @Keep
    @JvmStatic
    fun getPipStatusJson(): String {
      val act = instance ?: return org.json.JSONObject().put("supported", false).toString()
      if (Looper.myLooper() == Looper.getMainLooper()) {
        return act.buildPipStatusJson()
      }
      var result = act.buildPipStatusJson()
      val latch = CountDownLatch(1)
      act.runOnUiThread {
        try {
          result = act.buildPipStatusJson()
        } finally {
          latch.countDown()
        }
      }
      latch.await(1000, TimeUnit.MILLISECONDS)
      return result
    }

    @Keep
    @JvmStatic
    fun configurePip(
      enabled: Boolean,
      autoEnter: Boolean,
      hudMode: String,
      showButton: Boolean,
      inRide: Boolean
    ): String {
      val act = instance ?: return org.json.JSONObject().put("supported", false).toString()
      if (Looper.myLooper() == Looper.getMainLooper()) {
        act.applyPipConfig(enabled, autoEnter, hudMode, showButton, inRide)
        return act.buildPipStatusJson()
      }
      var result = act.buildPipStatusJson()
      val latch = CountDownLatch(1)
      act.runOnUiThread {
        try {
          act.applyPipConfig(enabled, autoEnter, hudMode, showButton, inRide)
          result = act.buildPipStatusJson()
        } finally {
          latch.countDown()
        }
      }
      latch.await(1500, TimeUnit.MILLISECONDS)
      return result
    }

    @Keep
    @JvmStatic
    fun enterPipModeJni(): Boolean {
      val act = instance ?: return false
      if (Looper.myLooper() == Looper.getMainLooper()) {
        return act.enterPipMode()
      }
      var entered = false
      val latch = CountDownLatch(1)
      act.runOnUiThread {
        try {
          entered = act.enterPipMode()
        } finally {
          latch.countDown()
        }
      }
      latch.await(1500, TimeUnit.MILLISECONDS)
      return entered
    }

    @Keep
    @JvmStatic
    fun getDualScreenStatusJson(): String {
      val act = instance ?: return org.json.JSONObject().put("supported", false).toString()
      if (Looper.myLooper() == Looper.getMainLooper()) {
        return act.buildDualStatusJson()
      }
      var result = act.buildDualStatusJson()
      val latch = CountDownLatch(1)
      act.runOnUiThread {
        try {
          result = act.buildDualStatusJson()
        } finally {
          latch.countDown()
        }
      }
      latch.await(1000, TimeUnit.MILLISECONDS)
      return result
    }

    @Keep
    @JvmStatic
    fun configureDualScreen(
      enabled: Boolean,
      mode: String,
      keepNameplates: Boolean,
      inRide: Boolean
    ): String {
      val act = instance ?: return org.json.JSONObject().put("supported", false).toString()
      if (Looper.myLooper() == Looper.getMainLooper()) {
        act.applyDualConfig(enabled, mode, keepNameplates, inRide)
        return act.buildDualStatusJson()
      }
      var result = act.buildDualStatusJson()
      val latch = CountDownLatch(1)
      act.runOnUiThread {
        try {
          act.applyDualConfig(enabled, mode, keepNameplates, inRide)
          result = act.buildDualStatusJson()
        } finally {
          latch.countDown()
        }
      }
      latch.await(1500, TimeUnit.MILLISECONDS)
      return result
    }

    @Keep
    @JvmStatic
    fun pushDualScreenPayload(payload: String): Boolean {
      val act = instance ?: return false
      act.runOnUiThread {
        act.dualPresentation?.pushPayload(payload)
      }
      return true
    }

    @Keep
    @JvmStatic
    fun startDataDirPicker() {
      dataDirResult = "pending"
      instance?.runOnUiThread {
        val act = instance ?: return@runOnUiThread
        act.suppressAutoPipUntilMs = SystemClock.elapsedRealtime() + 5000L
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && !Environment.isExternalStorageManager()) {
          try {
            val intent = Intent(
              Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
              Uri.parse("package:${act.packageName}")
            )
            act.storagePermLauncher?.launch(intent)
          } catch (_: Exception) {
            try {
              val intent = Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
              act.storagePermLauncher?.launch(intent)
            } catch (e: Exception) {
              dataDirResult = "error:Could not open storage permission settings: ${e.message}"
            }
          }
        } else {
          act.dataDirLauncher?.launch(null)
        }
      }
    }

    @Keep
    @JvmStatic
    fun getDataDirPickResult(): String {
      return dataDirResult
    }

    @Keep
    @JvmStatic
    fun resetDataDirPickResult() {
      dataDirResult = "pending"
    }

    @Keep
    @JvmStatic
    fun hasAllFilesAccess(): Boolean {
      return Build.VERSION.SDK_INT < Build.VERSION_CODES.R || Environment.isExternalStorageManager()
    }

    @Keep
    @JvmStatic
    fun getInternalFilesDir(): String {
      return instance?.filesDir?.absolutePath ?: ""
    }

    private var prevProcCpuMs: Long = -1L
    private var prevProcWallMs: Long = -1L

    @Keep
    @JvmStatic
    fun getPerfStatsJson(): String {
      val obj = org.json.JSONObject()
      val act = instance ?: return obj.toString()

      try {
        val nowCpuMs = Process.getElapsedCpuTime()
        val nowWallMs = SystemClock.elapsedRealtime()
        if (prevProcCpuMs >= 0L && prevProcWallMs >= 0L && nowWallMs > prevProcWallMs) {
          val dCpu = (nowCpuMs - prevProcCpuMs).coerceAtLeast(0L)
          val dWall = nowWallMs - prevProcWallMs
          val cores = Runtime.getRuntime().availableProcessors().coerceAtLeast(1)
          val pct = ((dCpu.toDouble() / dWall.toDouble()) * 100.0 / cores.toDouble()).coerceIn(0.0, 100.0)
          obj.put("appCpuPct", Math.round(pct * 10.0) / 10.0)
        }
        prevProcCpuMs = nowCpuMs
        prevProcWallMs = nowWallMs
      } catch (_: Exception) {}

      try {
        val am = act.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager
        if (am != null) {
          val mi = ActivityManager.MemoryInfo()
          am.getMemoryInfo(mi)
          val totalMb = mi.totalMem / (1024.0 * 1024.0)
          val usedMb = (mi.totalMem - mi.availMem).coerceAtLeast(0L) / (1024.0 * 1024.0)
          obj.put("memTotalMb", Math.round(totalMb * 10.0) / 10.0)
          obj.put("memUsedMb", Math.round(usedMb * 10.0) / 10.0)
        }
      } catch (_: Exception) {}

      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        try {
          val pm = act.getSystemService(Context.POWER_SERVICE) as? PowerManager
          if (pm != null) {
            val statusStr = when (pm.currentThermalStatus) {
              PowerManager.THERMAL_STATUS_NONE -> "none"
              PowerManager.THERMAL_STATUS_LIGHT -> "light"
              PowerManager.THERMAL_STATUS_MODERATE -> "moderate"
              PowerManager.THERMAL_STATUS_SEVERE -> "severe"
              PowerManager.THERMAL_STATUS_CRITICAL -> "critical"
              PowerManager.THERMAL_STATUS_EMERGENCY -> "emergency"
              PowerManager.THERMAL_STATUS_SHUTDOWN -> "shutdown"
              else -> null
            }
            if (statusStr != null) obj.put("thermalStatus", statusStr)
          }
        } catch (_: Exception) {}
      }

      try {
        val battIntent = act.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val rawTemp = battIntent?.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, Int.MIN_VALUE) ?: Int.MIN_VALUE
        if (rawTemp in -500..1500) {
          obj.put("batteryTempC", rawTemp / 10.0)
        }
      } catch (_: Exception) {}

      try {
        val kgsl = File("/sys/class/kgsl/kgsl-3d0/gpu_busy_percentage")
        if (kgsl.canRead()) {
          val pct = kgsl.readText().replace("%", "").trim().toDoubleOrNull()
          if (pct != null && pct in 0.0..100.0) {
            obj.put("gpuPct", Math.round(pct * 10.0) / 10.0)
          }
        }
      } catch (_: Exception) {}

      try {
        var cpuTemp: Double? = null
        var gpuTemp: Double? = null
        for (i in 0 until 24) {
          val dir = File("/sys/class/thermal/thermal_zone$i")
          val typeFile = File(dir, "type")
          val tempFile = File(dir, "temp")
          if (!typeFile.canRead() || !tempFile.canRead()) continue
          val tname = typeFile.readText().trim().lowercase()
          val raw = tempFile.readText().trim().toDoubleOrNull() ?: continue
          val c = if (raw > 1000.0) raw / 1000.0 else raw
          if (c !in 0.0..130.0) continue
          if (cpuTemp == null && (tname.contains("cpu") || tname.contains("soc") || tname.contains("tsens"))) {
            cpuTemp = Math.round(c * 10.0) / 10.0
          }
          if (gpuTemp == null && tname.contains("gpu")) {
            gpuTemp = Math.round(c * 10.0) / 10.0
          }
          if (cpuTemp != null && gpuTemp != null) break
        }
        if (cpuTemp != null) obj.put("cpuTempC", cpuTemp)
        if (gpuTemp != null) obj.put("gpuTempC", gpuTemp)
      } catch (_: Exception) {}

      return obj.toString()
    }
  }

  private var isFullscreen = true
  private var pipEnabled = false
  private var pipAutoEnter = true
  private var pipHudMode = "compact"
  private var pipShowButton = false
  private var dualEnabled = true
  private var dualMode = "cockpit"
  private var dualKeepNameplates = true
  private var dualPresentation: DualScreenPresentation? = null
  private var dualDisplayId: Int = Display.INVALID_DISPLAY
  private var displayManager: DisplayManager? = null
  private val displayListener = object : DisplayManager.DisplayListener {
    override fun onDisplayAdded(displayId: Int) {
      runOnUiThread {
        syncDualScreenPresentation()
        dispatchDualStateToWebView()
      }
    }

    override fun onDisplayChanged(displayId: Int) {
      runOnUiThread {
        syncDualScreenPresentation()
        dispatchDualStateToWebView()
      }
    }

    override fun onDisplayRemoved(displayId: Int) {
      runOnUiThread {
        if (displayId == dualDisplayId) {
          dismissDualPresentation()
        }
        syncDualScreenPresentation()
        dispatchDualStateToWebView()
      }
    }
  }
  private var inRideRoute = false
  private var activityResumed = false
  private var pendingPipEnter = false
  private var suppressAutoPipUntilMs = 0L
  private var dataDirLauncher: ActivityResultLauncher<Uri?>? = null
  private var storagePermLauncher: ActivityResultLauncher<Intent>? = null
  private val btAppMarker = Regex("""(?<![.\w$])this\.riderController\.handleInput\(\s*\)""")
  private val patchedChunks = ConcurrentHashMap<String, ByteArray>()
  private val unmatchedChunks = ConcurrentHashMap.newKeySet<String>()
  private var btAppWrappedClient: WebViewClient? = null
  private var wrappedChromeClient: WebChromeClient? = null
  private var pipLayoutTrackedWebView: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    instance = this
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

    loadPipPrefs()
    loadDualPrefs()
    displayManager = getSystemService(Context.DISPLAY_SERVICE) as? DisplayManager
    displayManager?.registerDisplayListener(displayListener, Handler(Looper.getMainLooper()))

    dataDirLauncher = registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
      handleDataDirPicked(uri)
    }
    storagePermLauncher = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) {
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && !Environment.isExternalStorageManager()) {
        dataDirResult = "error:All files access permission is required to store Terra data in a custom folder."
      } else {
        dataDirLauncher?.launch(null)
      }
    }

    val cookieManager = CookieManager.getInstance()
    cookieManager.setAcceptCookie(true)

    window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
      setShowWhenLocked(true)
      setTurnScreenOn(true)
    }

    extractBundledAddons()

    ViewCompat.setOnApplyWindowInsetsListener(window.decorView) { view, insets ->
      val inPip = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && isInPictureInPictureMode
      if (!isFullscreen && !inPip) {
        val statusBarHeight = insets.getInsets(WindowInsetsCompat.Type.statusBars()).top
        val navBarHeight = insets.getInsets(WindowInsetsCompat.Type.navigationBars()).bottom
        view.setPadding(0, statusBarHeight, 0, navBarHeight)
      } else {
        view.setPadding(0, 0, 0, 0)
      }
      insets
    }

    toggleFullscreen(true)

    BleManager.init(this)
    requestBlePermissions()

    window.decorView.post {
      setupWebViewFullscreen()
      updatePictureInPictureParams()
    }
  }

  override fun onResume() {
    super.onResume()
    activityResumed = true
    pendingPipEnter = false
    suppressAutoPipUntilMs = 0L
    updatePictureInPictureParams()
    syncDualScreenPresentation()
    dispatchDualStateToWebView()
  }

  override fun onPause() {
    super.onPause()
    try {
      CookieManager.getInstance().flush()
    } catch (_: Exception) {}

    val inPip = (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && isInPictureInPictureMode) || pendingPipEnter
    if (inPip) {
      // WryActivity.onPause() unconditionally calls mWebView.onPause(), which freezes
      // WebGL/WebGPU and JS timers while the Activity is still visible in PiP.
      findWebView(window.decorView)?.let { wv ->
        wv.onResume()
        wv.resumeTimers()
      }
    }
  }

  override fun onStop() {
    super.onStop()
    activityResumed = false
    pendingPipEnter = false
    dismissDualPresentation()
    val inPip = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && isInPictureInPictureMode
    if (!inPip) {
      findWebView(window.decorView)?.onPause()
    }
  }

  override fun onUserLeaveHint() {
    super.onUserLeaveHint()
    if (Build.VERSION.SDK_INT in Build.VERSION_CODES.O until Build.VERSION_CODES.S) {
      if (SystemClock.elapsedRealtime() < suppressAutoPipUntilMs) return
      if (pipEnabled && pipAutoEnter && inRideRoute && !isInPictureInPictureMode) {
        enterPipMode()
      }
    }
  }

  override fun onPictureInPictureUiStateChanged(pipState: android.app.PictureInPictureUiState) {
    super.onPictureInPictureUiStateChanged(pipState)
    if (Build.VERSION.SDK_INT >= 35 && pipState.isTransitioningToPip) {
      pendingPipEnter = true
      window.decorView.setPadding(0, 0, 0, 0)
      dispatchPipStateToWebView(true)
    }
  }

  override fun onPictureInPictureModeChanged(
    isInPictureInPictureMode: Boolean,
    newConfig: Configuration
  ) {
    super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
    pendingPipEnter = false
    if (isInPictureInPictureMode) {
      window.decorView.setPadding(0, 0, 0, 0)
      findWebView(window.decorView)?.let { wv ->
        wv.onResume()
        wv.resumeTimers()
      }
      window.decorView.post {
        findWebView(window.decorView)?.let { wv ->
          wv.onResume()
          wv.resumeTimers()
        }
      }
    } else {
      toggleFullscreen(isFullscreen)
    }
    dispatchPipStateToWebView(isInPictureInPictureMode)
    syncDualScreenPresentation()
    dispatchDualStateToWebView()
  }

  override fun onDestroy() {
    dismissDualPresentation()
    try {
      displayManager?.unregisterDisplayListener(displayListener)
    } catch (_: Exception) {}
    TerraBleService.stop(this)
    try {
      CookieManager.getInstance().flush()
    } catch (_: Exception) {}
    super.onDestroy()
  }

  private fun copyAssetTree(assetPath: String, target: File) {
    val children = assets.list(assetPath)
    if (children != null && children.isNotEmpty()) {
      target.mkdirs()
      for (child in children) {
        copyAssetTree("$assetPath/$child", File(target, child))
      }
    } else {
      target.parentFile?.mkdirs()
      assets.open(assetPath).use { inStream ->
        FileOutputStream(target).use { outStream ->
          inStream.copyTo(outStream)
        }
      }
    }
  }

  private fun extractBundledAddons() {
    val dataDir = getTerraDataDir()
    val addonsDir = File(dataDir, "addons")
    if (!addonsDir.exists()) addonsDir.mkdirs()

    // keep in sync with BUNDLED_ADDONS in build-apk.sh
    val bundledAddons = listOf("zwift-ride", "garmin-hud", "fps-monitor", "wifi-trainer-bridge")
    for (addonName in bundledAddons) {
      try {
        val targetDir = File(addonsDir, addonName)
        if (!shouldExtract(addonName, targetDir)) continue
        if (targetDir.exists()) targetDir.deleteRecursively() // stale copy: replace it
        copyAssetTree("addons/$addonName", targetDir)
        android.util.Log.i(
          "Terra",
          "Extracted bundled $addonName addon v${bundledAddonVersion(addonName) ?: "?"}"
        )
      } catch (e: Exception) {
        android.util.Log.e("Terra", "Error extracting bundled addon $addonName", e)
      }
    }
  }

  /// Re-extract when the on-device copy is missing, has no manifest (incomplete),
  /// or is OLDER than the version bundled in the APK. A copy that is the same or
  /// newer (e.g. hand-updated on the device) is left untouched.
  private fun shouldExtract(addonName: String, targetDir: File): Boolean {
    if (!targetDir.exists()) return true
    val assetVersion = bundledAddonVersion(addonName) ?: return false
    val diskVersion = diskAddonVersion(targetDir) ?: return true
    return compareVersions(diskVersion, assetVersion) < 0
  }

  private fun bundledAddonVersion(addonName: String): String? = try {
    assets.open("addons/$addonName/manifest.json").bufferedReader().use { it.readText() }
      .let(::versionOf)
  } catch (e: Exception) {
    null
  }

  private fun diskAddonVersion(targetDir: File): String? {
    val manifest = File(targetDir, "manifest.json")
    if (!manifest.isFile) return null
    return runCatching { versionOf(manifest.readText()) }.getOrNull()
  }

  private fun versionOf(json: String): String? =
    Regex("\"version\"\\s*:\\s*\"([^\"]+)\"").find(json)?.groupValues?.get(1)

  /** Numeric-dots comparison: 0.4.0 < 0.4.1 < 0.10.0; ignores suffixes like -beta. */
  private fun compareVersions(a: String, b: String): Int {
    fun segments(v: String): List<Int> = v.split(Regex("[^0-9]+")).map { it.toIntOrNull() ?: 0 }
    val sa = segments(a)
    val sb = segments(b)
    for (i in 0 until maxOf(sa.size, sb.size)) {
      val diff = (sa.getOrElse(i) { 0 }).compareTo(sb.getOrElse(i) { 0 })
      if (diff != 0) return diff
    }
    return 0
  }

  // ─── Data directory picker ───────────────────────────────────────────────────

  /// Resolve the effective data directory: user-picked (pointer file) or default Android/data.
  private fun getTerraDataDir(): File {
    val pointer = File(filesDir, "terra-data-dir.txt")
    if (pointer.exists()) {
      val path = runCatching { pointer.readText().trim() }.getOrNull()
      if (!path.isNullOrEmpty()) {
        val f = File(path)
        if (f.exists() || f.mkdirs()) return f
      }
    }
    return getExternalFilesDir(null) ?: filesDir
  }

  private fun handleDataDirPicked(uri: Uri?) {
    if (uri == null) {
      dataDirResult = "cancelled"
      return
    }
    try {
      contentResolver.takePersistableUriPermission(
        uri,
        Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
      )
    } catch (e: Exception) {
      dataDirResult = "error:failed to get persistent access: ${e.message}"
      return
    }

    val path = uriToPath(uri)
    if (path == null) {
      dataDirResult = "error:could not resolve a filesystem path (pick a folder on main storage)"
      return
    }

    // Probe: verify the path is actually writable
    try {
      val dir = File(path)
      dir.mkdirs()
      val marker = File(dir, ".terra-probe")
      marker.writeText("ok")
      if (marker.readText() != "ok") throw Exception("read-back mismatch")
      marker.delete()
    } catch (e: Exception) {
      dataDirResult = "error:folder not writable: ${e.message}"
      return
    }

    // Migrate existing state from the old (default) location
    try {
      migrateData(File(path))
    } catch (e: Exception) {
      dataDirResult = "error:migration failed: ${e.message}"
      return
    }

    // Persist the pointer and mark first-launch prompt as completed
    try {
      File(filesDir, "terra-data-dir.txt").writeText(path)
      File(filesDir, "terra-datadir-prompted.txt").writeText("1")
    } catch (e: Exception) {
      dataDirResult = "error:failed to save preference: ${e.message}"
      return
    }

    android.util.Log.i("Terra", "Data dir set to: $path")
    dataDirResult = "success:$path"
  }

  /// Map a content:// tree URI to a real /storage/… path. Returns null for unresolvable URIs.
  private fun uriToPath(uri: Uri): String? {
    // content://com.android.externalstorage.documents/tree/primary%3ADocuments%2FTerra
    //   → /storage/emulated/0/Documents/Terra
    val docId = try {
      DocumentsContract.getTreeDocumentId(uri) ?: return null
    } catch (_: Exception) { return null }

    // docId looks like "primary:Documents/Terra" or "primary%3ADocuments%2FTerra"
    val decoded = Uri.decode(docId)
    val colonIdx = decoded.indexOf(':')
    if (colonIdx <= 0) return null
    val volume = decoded.substring(0, colonIdx)
    if (volume != "primary") return null // only main storage supported
    val subPath = decoded.substring(colonIdx + 1)
    return if (subPath.isEmpty()) "/storage/emulated/0" else "/storage/emulated/0/$subPath"
  }

  /// Copy existing addons/, addon-data/, devices.json from the old default dir to [newDir].
  /// Only copies files that don't already exist in [newDir] (non-destructive).
  private fun migrateData(newDir: File) {
    val oldDir = getTerraDataDir()
    if (oldDir == newDir) return
    if (!oldDir.exists()) return

    // Copy entire subdirectories
    for (sub in listOf("addons", "addon-data")) {
      val src = File(oldDir, sub)
      if (!src.exists()) continue
      val dst = File(newDir, sub)
      if (!dst.exists()) dst.mkdirs()
      copyDirTree(src, dst)
    }

    // Copy devices.json
    val devSrc = File(oldDir, "devices.json")
    if (devSrc.exists()) {
      val devDst = File(newDir, "devices.json")
      if (!devDst.exists()) devSrc.copyTo(devDst)
    }
  }

  private fun copyDirTree(src: File, dst: File) {
    if (src.isDirectory) {
      if (!dst.exists()) dst.mkdirs()
      src.listFiles()?.forEach { child -> copyDirTree(child, File(dst, child.name)) }
    } else if (src.isFile) {
      if (!dst.exists()) {
        dst.parentFile?.mkdirs()
        src.copyTo(dst)
      }
    }
  }

  fun toggleFullscreen(enable: Boolean? = null) {
    val target = enable ?: !isFullscreen
    isFullscreen = target
    val inPip = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && isInPictureInPictureMode
    val insetsController = WindowCompat.getInsetsController(window, window.decorView)
    if (target || inPip) {
      insetsController.hide(WindowInsetsCompat.Type.systemBars())
      insetsController.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
      window.decorView.setPadding(0, 0, 0, 0)
    } else {
      insetsController.show(WindowInsetsCompat.Type.systemBars())
      val insets = ViewCompat.getRootWindowInsets(window.decorView)
      val statusBarHeight = insets?.getInsets(WindowInsetsCompat.Type.statusBars())?.top ?: 0
      val navBarHeight = insets?.getInsets(WindowInsetsCompat.Type.navigationBars())?.bottom ?: 0
      window.decorView.setPadding(0, statusBarHeight, 0, navBarHeight)
    }
  }

  private fun isPipSupported(): Boolean =
    Build.VERSION.SDK_INT >= Build.VERSION_CODES.O &&
      packageManager.hasSystemFeature(PackageManager.FEATURE_PICTURE_IN_PICTURE)

  private fun loadPipPrefs() {
    val prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE)
    pipEnabled = prefs.getBoolean(PREF_PIP_ENABLED, false)
    pipAutoEnter = prefs.getBoolean(PREF_PIP_AUTO_ENTER, true)
    pipHudMode = "compact"
    pipShowButton = prefs.getBoolean(PREF_PIP_SHOW_BUTTON, false)
  }

  private fun applyPipConfig(
    enabled: Boolean,
    autoEnter: Boolean,
    hudMode: String,
    showButton: Boolean,
    inRide: Boolean
  ) {
    pipEnabled = enabled
    pipAutoEnter = autoEnter
    pipHudMode = "compact"
    pipShowButton = showButton
    inRideRoute = inRide
    getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
      .putBoolean(PREF_PIP_ENABLED, pipEnabled)
      .putBoolean(PREF_PIP_AUTO_ENTER, pipAutoEnter)
      .putString(PREF_PIP_HUD_MODE, pipHudMode)
      .putBoolean(PREF_PIP_SHOW_BUTTON, pipShowButton)
      .apply()
    updatePictureInPictureParams()
    syncDualScreenPresentation()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && isInPictureInPictureMode) {
      dispatchPipStateToWebView(true)
    }
  }

  private fun buildPipStatusJson(): String {
    val inPip = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && isInPictureInPictureMode
    return org.json.JSONObject()
      .put("supported", isPipSupported())
      .put("enabled", pipEnabled)
      .put("autoEnter", pipAutoEnter)
      .put("hudMode", pipHudMode)
      .put("showButton", pipShowButton)
      .put("inRide", inRideRoute)
      .put("inPip", inPip)
      .toString()
  }

  private fun buildPipParams(autoEnter: Boolean): PictureInPictureParams? {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return null
    val builder = PictureInPictureParams.Builder()
      .setAspectRatio(Rational(16, 9))
    findWebView(window.decorView)?.let { wv ->
      if (wv.width > 0 && wv.height > 0) {
        val rect = Rect()
        if (wv.getGlobalVisibleRect(rect) && !rect.isEmpty) {
          builder.setSourceRectHint(rect)
        }
      }
    }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      builder.setAutoEnterEnabled(autoEnter)
      builder.setSeamlessResizeEnabled(true)
    }
    return builder.build()
  }

  private fun updatePictureInPictureParams() {
    if (!isPipSupported() || Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
    val auto = pipEnabled && pipAutoEnter && inRideRoute
    val params = buildPipParams(auto) ?: return
    try {
      setPictureInPictureParams(params)
    } catch (_: Exception) {}
  }

  private fun enterPipMode(): Boolean {
    if (!isPipSupported() || !pipEnabled || Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
      return false
    }
    if (isInPictureInPictureMode) return true
    val params = buildPipParams(pipEnabled && pipAutoEnter && inRideRoute) ?: return false
    pendingPipEnter = true
    val ok = try {
      enterPictureInPictureMode(params)
    } catch (e: Exception) {
      android.util.Log.w("Terra", "enterPictureInPictureMode failed: ${e.message}")
      false
    }
    if (!ok) {
      pendingPipEnter = false
    } else {
      dispatchPipStateToWebView(true)
    }
    return ok
  }

  private fun isRideUrl(rawUrl: String?): Boolean {
    if (rawUrl.isNullOrBlank()) return false
    return try {
      val uri = Uri.parse(rawUrl)
      val host = uri.host?.lowercase() ?: return false
      val path = uri.path ?: ""
      (host == "biketerra.com" || host.endsWith(".biketerra.com")) &&
        (path == "/ride" || path.startsWith("/ride/") || path == "/spectate" || path.startsWith("/spectate/"))
    } catch (_: Exception) {
      false
    }
  }

  private fun updateRideStateFromUrl(url: String?) {
    val next = isRideUrl(url)
    if (next != inRideRoute) {
      inRideRoute = next
      updatePictureInPictureParams()
      syncDualScreenPresentation()
      dispatchDualStateToWebView()
    }
  }

  // ─── Dual-Screen (Secondary Display Presentation) ──────────────────────────

  private fun loadDualPrefs() {
    val prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE)
    dualEnabled = prefs.getBoolean(PREF_DUAL_ENABLED, true)
    dualMode = when (prefs.getString(PREF_DUAL_MODE, "cockpit")) {
      "mirror" -> "mirror"
      else -> "cockpit"
    }
    dualKeepNameplates = prefs.getBoolean(PREF_DUAL_KEEP_NAMEPLATES, true)
  }

  private fun findSecondaryDisplay(): Display? {
    val dm = displayManager ?: (getSystemService(Context.DISPLAY_SERVICE) as? DisplayManager) ?: return null
    val pres = try {
      dm.getDisplays(DisplayManager.DISPLAY_CATEGORY_PRESENTATION)
    } catch (_: Exception) {
      emptyArray()
    }
    pres.firstOrNull {
      it.displayId != Display.DEFAULT_DISPLAY && it.isValid && it.state != Display.STATE_OFF
    }?.let { return it }

    return dm.displays.firstOrNull {
      it.displayId != Display.DEFAULT_DISPLAY && it.isValid && it.state != Display.STATE_OFF
    }
  }

  private fun readDisplaySize(display: Display): Pair<Int, Int> {
    return try {
      val ctx = createDisplayContext(display)
      val dm = ctx.resources.displayMetrics
      if (dm.widthPixels > 0 && dm.heightPixels > 0) {
        Pair(dm.widthPixels, dm.heightPixels)
      } else {
        val pt = Point()
        @Suppress("DEPRECATION")
        display.getRealSize(pt)
        Pair(pt.x, pt.y)
      }
    } catch (_: Exception) {
      Pair(0, 0)
    }
  }

  private fun isDualScreenActiveNow(secondary: Display? = findSecondaryDisplay()): Boolean {
    val inPip = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && isInPictureInPictureMode
    return secondary != null && dualEnabled && inRideRoute && activityResumed && !inPip && !pendingPipEnter
  }

  private fun applyDualConfig(
    enabled: Boolean,
    mode: String,
    keepNameplates: Boolean,
    inRide: Boolean
  ) {
    dualEnabled = enabled
    dualMode = if (mode == "mirror") "mirror" else "cockpit"
    dualKeepNameplates = keepNameplates
    inRideRoute = inRide
    getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
      .putBoolean(PREF_DUAL_ENABLED, dualEnabled)
      .putString(PREF_DUAL_MODE, dualMode)
      .putBoolean(PREF_DUAL_KEEP_NAMEPLATES, dualKeepNameplates)
      .apply()
    updatePictureInPictureParams()
    syncDualScreenPresentation()
    dispatchDualStateToWebView()
  }

  private fun buildDualStatusJson(): String {
    val sec = findSecondaryDisplay()
    val (w, h) = if (sec != null) readDisplaySize(sec) else Pair(0, 0)
    val active = isDualScreenActiveNow(sec) && dualPresentation != null
    return org.json.JSONObject()
      .put("supported", true)
      .put("hasSecondaryDisplay", sec != null)
      .put("displayId", sec?.displayId ?: Display.INVALID_DISPLAY)
      .put("displayName", sec?.name ?: "")
      .put("displayWidth", w)
      .put("displayHeight", h)
      .put("enabled", dualEnabled)
      .put("mode", dualMode)
      .put("keepNameplates", dualKeepNameplates)
      .put("inRide", inRideRoute)
      .put("active", active)
      .toString()
  }

  private fun dismissDualPresentation() {
    val pres = dualPresentation ?: return
    dualPresentation = null
    dualDisplayId = Display.INVALID_DISPLAY
    try {
      if (pres.isShowing) pres.dismiss()
    } catch (_: Exception) {}
  }

  private fun syncDualScreenPresentation() {
    if (isFinishing || isDestroyed) {
      dismissDualPresentation()
      return
    }
    val sec = findSecondaryDisplay()
    if (!isDualScreenActiveNow(sec) || sec == null) {
      dismissDualPresentation()
      return
    }
    if (dualPresentation != null && dualDisplayId == sec.displayId && dualPresentation?.isShowing == true) {
      dualPresentation?.updateState(buildDualStatusJson())
      return
    }
    dismissDualPresentation()
    try {
      val pres = DualScreenPresentation(
        this,
        sec,
        onAction = { json -> handleDualScreenAction(json) },
        onPageReady = {
          runOnUiThread {
            dualPresentation?.updateState(buildDualStatusJson())
            dispatchDualStateToWebView()
          }
        }
      )
      pres.show()
      dualPresentation = pres
      dualDisplayId = sec.displayId
      pres.updateState(buildDualStatusJson())
      android.util.Log.i("Terra", "DualScreenPresentation active on display ${sec.displayId} (${sec.name})")
    } catch (e: Exception) {
      android.util.Log.w("Terra", "Failed to show DualScreenPresentation: ${e.message}")
      dismissDualPresentation()
    }
  }

  private fun handleDualScreenAction(actionJson: String) {
    runOnUiThread {
      try {
        val obj = org.json.JSONObject(actionJson)
        if (obj.optString("type") == "setMode") {
          val nextMode = if (obj.optString("mode") == "mirror") "mirror" else "cockpit"
          applyDualConfig(dualEnabled, nextMode, dualKeepNameplates, inRideRoute)
          return@runOnUiThread
        }
      } catch (_: Exception) {}

      val wv = findWebView(window.decorView) ?: return@runOnUiThread
      val quoted = org.json.JSONObject.quote(actionJson)
      wv.post {
        try {
          wv.evaluateJavascript(
            "window.__terraDualAction && window.__terraDualAction(JSON.parse($quoted));",
            null
          )
        } catch (_: Exception) {}
      }
    }
  }

  private fun dispatchDualStateToWebView() {
    val wv = findWebView(window.decorView) ?: return
    val statusQuoted = org.json.JSONObject.quote(buildDualStatusJson())
    val js = "window.__terraSetDualScreenState && window.__terraSetDualScreenState(JSON.parse($statusQuoted));"
    wv.post {
      try {
        wv.evaluateJavascript(js, null)
      } catch (_: Exception) {}
    }
  }

  private fun dispatchPipStateToWebView(inPip: Boolean) {
    val wv = findWebView(window.decorView) ?: return
    val modeQuoted = org.json.JSONObject.quote(pipHudMode)
    val js = "window.__terraSetPipState && window.__terraSetPipState(${if (inPip) "true" else "false"}, $modeQuoted);"
    wv.post {
      try {
        wv.evaluateJavascript(js, null)
      } catch (_: Exception) {}
    }
  }

  private fun trackPipLayoutOnWebView(webView: WebView) {
    if (pipLayoutTrackedWebView === webView) return
    pipLayoutTrackedWebView = webView
    webView.addOnLayoutChangeListener { _, left, top, right, bottom, oldLeft, oldTop, oldRight, oldBottom ->
      if (left != oldLeft || top != oldTop || right != oldRight || bottom != oldBottom) {
        updatePictureInPictureParams()
      }
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    clearBtAppChunkCacheOnce(webView)
  }

  override fun setContentView(view: View?) {
    super.setContentView(view)
    val wv = view as? WebView ?: findWebView(window.decorView)
    if (wv != null) {
      trackPipLayoutOnWebView(wv)
      installBtAppChunkInterceptor(wv)
    }
  }

  private fun clearBtAppChunkCacheOnce(webView: WebView) {
    val prefs = getSharedPreferences("terra_prefs", MODE_PRIVATE)
    val needBtAppClear = !prefs.getBoolean("btapp_cache_cleared_v2", false)
    val needCssCompatClear = CssCompat.needsColorCompat(this) &&
      !prefs.getBoolean("css_compat_cache_cleared_v1", false)
    if (needBtAppClear || needCssCompatClear) {
      try {
        webView.clearCache(true)
        prefs.edit().apply {
          if (needBtAppClear) putBoolean("btapp_cache_cleared_v2", true)
          if (needCssCompatClear) putBoolean("css_compat_cache_cleared_v1", true)
        }.apply()
        android.util.Log.i(
          "Terra",
          "Cleared WebView cache (btApp=$needBtAppClear, cssCompat=$needCssCompatClear, wv=${CssCompat.getWebViewMajorVersion(this)})"
        )
      } catch (_: Exception) {}
    }
  }

  private fun installBtAppChunkInterceptor(webView: WebView) {
    clearBtAppChunkCacheOnce(webView)

    if (!WebViewFeature.isFeatureSupported(WebViewFeature.GET_WEB_VIEW_CLIENT)) {
      android.util.Log.w("Terra", "GET_WEB_VIEW_CLIENT not supported; window.btApp auto-hook disabled")
      return
    }

    val delegate = try {
      WebViewCompat.getWebViewClient(webView)
    } catch (e: Exception) {
      android.util.Log.w("Terra", "Could not read existing WebViewClient: ${e.message}")
      return
    }

    if (delegate === btAppWrappedClient) return

    val wrapper = object : WebViewClient() {
      override fun shouldInterceptRequest(
        view: WebView,
        request: WebResourceRequest
      ): WebResourceResponse? {
        delegate.shouldInterceptRequest(view, request)?.let { return it }
        interceptBiketerraChunk(request)?.let { return it }
        CssCompat.interceptCssIfNeeded(this@MainActivity, request)?.let { return it }
        return null
      }

      override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
        delegate.shouldOverrideUrlLoading(view, request)

      override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
        updateRideStateFromUrl(url)
        delegate.onPageStarted(view, url, favicon)
      }

      override fun onPageFinished(view: WebView, url: String) {
        updateRideStateFromUrl(url)
        delegate.onPageFinished(view, url)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && isInPictureInPictureMode) {
          dispatchPipStateToWebView(true)
        }
        dispatchDualStateToWebView()
      }

      override fun onPageCommitVisible(view: WebView, url: String) {
        updateRideStateFromUrl(url)
        delegate.onPageCommitVisible(view, url)
      }

      override fun onLoadResource(view: WebView, url: String) {
        delegate.onLoadResource(view, url)
      }

      override fun doUpdateVisitedHistory(view: WebView, url: String, isReload: Boolean) {
        updateRideStateFromUrl(url)
        delegate.doUpdateVisitedHistory(view, url, isReload)
      }

      override fun onReceivedError(
        view: WebView,
        request: WebResourceRequest,
        error: WebResourceError
      ) {
        delegate.onReceivedError(view, request, error)
      }

      override fun onReceivedHttpError(
        view: WebView,
        request: WebResourceRequest,
        errorResponse: WebResourceResponse
      ) {
        delegate.onReceivedHttpError(view, request, errorResponse)
      }

      override fun onReceivedSslError(
        view: WebView,
        handler: SslErrorHandler,
        error: SslError
      ) {
        delegate.onReceivedSslError(view, handler, error)
      }

      override fun onReceivedHttpAuthRequest(
        view: WebView,
        handler: HttpAuthHandler,
        host: String,
        realm: String
      ) {
        delegate.onReceivedHttpAuthRequest(view, handler, host, realm)
      }

      override fun onReceivedClientCertRequest(view: WebView, request: ClientCertRequest) {
        delegate.onReceivedClientCertRequest(view, request)
      }

      override fun onFormResubmission(view: WebView, dontResend: Message, resend: Message) {
        delegate.onFormResubmission(view, dontResend, resend)
      }

      override fun onScaleChanged(view: WebView, oldScale: Float, newScale: Float) {
        delegate.onScaleChanged(view, oldScale, newScale)
      }

      override fun shouldOverrideKeyEvent(view: WebView, event: KeyEvent): Boolean =
        delegate.shouldOverrideKeyEvent(view, event)

      override fun onUnhandledKeyEvent(view: WebView, event: KeyEvent) {
        delegate.onUnhandledKeyEvent(view, event)
      }
    }
    btAppWrappedClient = wrapper
    webView.webViewClient = wrapper
    android.util.Log.i(
      "Terra",
      "Installed window.btApp chunk interceptor (delegate=${delegate.javaClass.simpleName})"
    )
  }

  private fun interceptBiketerraChunk(request: WebResourceRequest): WebResourceResponse? {
    if (!"GET".equals(request.method, ignoreCase = true)) return null
    val uri = request.url ?: return null
    if (!"https".equals(uri.scheme, ignoreCase = true)) return null
    val host = uri.host?.lowercase() ?: return null
    if (host != "biketerra.com" && host != "www.biketerra.com") return null
    val path = uri.path ?: return null
    if (!path.startsWith("/_app/immutable/chunks/") || !path.endsWith(".js")) return null

    val urlStr = uri.toString()
    patchedChunks[urlStr]?.let { bytes ->
      return jsChunkResponse(bytes, request, cacheControl = "no-store")
    }
    if (unmatchedChunks.contains(urlStr)) return null

    var conn: HttpURLConnection? = null
    val rawBytes = try {
      conn = (URI(urlStr).toURL().openConnection() as HttpURLConnection).apply {
        requestMethod = "GET"
        instanceFollowRedirects = true
        connectTimeout = 5000
        readTimeout = 10000
        val ua = request.requestHeaders?.entries
          ?.firstOrNull { it.key.equals("user-agent", ignoreCase = true) }
          ?.value
        if (!ua.isNullOrEmpty()) setRequestProperty("User-Agent", ua)
      }
      if (conn.responseCode != HttpURLConnection.HTTP_OK) {
        return null
      }
      conn.inputStream.use { it.readBytes() }
    } catch (e: Exception) {
      android.util.Log.w("Terra", "Chunk inspect failed for $urlStr: ${e.message}")
      return null
    } finally {
      conn?.disconnect()
    }

    val src = String(rawBytes, Charsets.UTF_8)
    if (!btAppMarker.containsMatchIn(src)) {
      unmatchedChunks.add(urlStr)
      return jsChunkResponse(rawBytes, request, cacheControl = "public, max-age=31536000, immutable")
    }

    val patched = btAppMarker.replace(src) { m ->
      "(typeof window!==\"undefined\"&&(window.btApp=this),${m.value})"
    }
    val patchedBytes = patched.toByteArray(Charsets.UTF_8)
    patchedChunks[urlStr] = patchedBytes
    android.util.Log.i("Terra", "Injected window.btApp hook into ${path.substringAfterLast('/')}")
    return jsChunkResponse(patchedBytes, request, cacheControl = "no-store")
  }

  private fun jsChunkResponse(
    bytes: ByteArray,
    request: WebResourceRequest,
    cacheControl: String
  ): WebResourceResponse {
    val origin = request.requestHeaders?.entries
      ?.firstOrNull { it.key.equals("origin", ignoreCase = true) }
      ?.value
    val headers = mutableMapOf(
      "Content-Type" to "application/javascript; charset=utf-8",
      "Cache-Control" to cacheControl,
      "Access-Control-Allow-Origin" to (origin ?: "https://biketerra.com"),
      "Vary" to "Origin"
    )
    if (origin != null) {
      headers["Access-Control-Allow-Credentials"] = "true"
    }
    return WebResourceResponse(
      "application/javascript",
      "utf-8",
      200,
      "OK",
      headers,
      ByteArrayInputStream(bytes)
    )
  }

  private fun setupWebViewFullscreen() {
    val wv = findWebView(window.decorView) ?: return
    trackPipLayoutOnWebView(wv)
    installBtAppChunkInterceptor(wv)
    wv.keepScreenOn = true
    try {
      CookieManager.getInstance().setAcceptThirdPartyCookies(wv, true)
    } catch (_: Exception) {}

    val delegate = try {
      when {
        WebViewFeature.isFeatureSupported(WebViewFeature.GET_WEB_CHROME_CLIENT) ->
          WebViewCompat.getWebChromeClient(wv)
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.O ->
          wv.webChromeClient
        else -> null
      }
    } catch (_: Exception) {
      null
    }

    if (delegate != null && delegate === wrappedChromeClient) return

    val chromeWrapper = object : WebChromeClient() {
      override fun onShowCustomView(view: View?, callback: CustomViewCallback?) {
        toggleFullscreen(true)
      }

      override fun onHideCustomView() {
        toggleFullscreen(false)
      }

      override fun onPermissionRequest(request: PermissionRequest) {
        if (delegate != null) delegate.onPermissionRequest(request)
        else super.onPermissionRequest(request)
      }

      override fun onPermissionRequestCanceled(request: PermissionRequest) {
        if (delegate != null) delegate.onPermissionRequestCanceled(request)
        else super.onPermissionRequestCanceled(request)
      }

      override fun onGeolocationPermissionsShowPrompt(
        origin: String,
        callback: GeolocationPermissions.Callback
      ) {
        if (delegate != null) delegate.onGeolocationPermissionsShowPrompt(origin, callback)
        else super.onGeolocationPermissionsShowPrompt(origin, callback)
      }

      override fun onGeolocationPermissionsHidePrompt() {
        if (delegate != null) delegate.onGeolocationPermissionsHidePrompt()
        else super.onGeolocationPermissionsHidePrompt()
      }

      override fun onJsAlert(
        view: WebView,
        url: String,
        message: String,
        result: JsResult
      ): Boolean =
        delegate?.onJsAlert(view, url, message, result)
          ?: super.onJsAlert(view, url, message, result)

      override fun onJsConfirm(
        view: WebView,
        url: String,
        message: String,
        result: JsResult
      ): Boolean =
        delegate?.onJsConfirm(view, url, message, result)
          ?: super.onJsConfirm(view, url, message, result)

      override fun onJsPrompt(
        view: WebView,
        url: String,
        message: String,
        defaultValue: String,
        result: JsPromptResult
      ): Boolean =
        delegate?.onJsPrompt(view, url, message, defaultValue, result)
          ?: super.onJsPrompt(view, url, message, defaultValue, result)

      override fun onShowFileChooser(
        webView: WebView,
        filePathCallback: ValueCallback<Array<Uri?>?>,
        fileChooserParams: FileChooserParams
      ): Boolean =
        delegate?.onShowFileChooser(webView, filePathCallback, fileChooserParams)
          ?: super.onShowFileChooser(webView, filePathCallback, fileChooserParams)

      override fun onConsoleMessage(consoleMessage: ConsoleMessage): Boolean {
        if (consoleMessage.messageLevel() == ConsoleMessage.MessageLevel.ERROR) {
          android.util.Log.e(
            "TerraWeb",
            "${consoleMessage.message()} (${consoleMessage.sourceId()}:${consoleMessage.lineNumber()})"
          )
        }
        return delegate?.onConsoleMessage(consoleMessage)
          ?: super.onConsoleMessage(consoleMessage)
      }

      override fun onReceivedTitle(view: WebView, title: String) {
        if (delegate != null) delegate.onReceivedTitle(view, title)
        else super.onReceivedTitle(view, title)
      }

      override fun getDefaultVideoPoster(): Bitmap? =
        delegate?.defaultVideoPoster ?: super.getDefaultVideoPoster()

      override fun onProgressChanged(view: WebView, newProgress: Int) {
        if (delegate != null) delegate.onProgressChanged(view, newProgress)
        else super.onProgressChanged(view, newProgress)
      }
    }
    wrappedChromeClient = chromeWrapper
    wv.webChromeClient = chromeWrapper
  }

  private fun findWebView(view: View): WebView? {
    if (view is WebView) return view
    if (view is ViewGroup) {
      for (i in 0 until view.childCount) {
        val wv = findWebView(view.getChildAt(i))
        if (wv != null) return wv
      }
    }
    return null
  }

  private fun requestBlePermissions() {
    val permissions = mutableListOf<String>()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      if (ContextCompat.checkSelfPermission(this, Manifest.permission.BLUETOOTH_SCAN) != PackageManager.PERMISSION_GRANTED) {
        permissions.add(Manifest.permission.BLUETOOTH_SCAN)
      }
      if (ContextCompat.checkSelfPermission(this, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
        permissions.add(Manifest.permission.BLUETOOTH_CONNECT)
      }
    }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
        permissions.add(Manifest.permission.POST_NOTIFICATIONS)
      }
    }
    if (Build.VERSION.SDK_INT >= 37) {
      if (ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_LOCAL_NETWORK) != PackageManager.PERMISSION_GRANTED) {
        permissions.add(Manifest.permission.ACCESS_LOCAL_NETWORK)
      }
    }
    if (ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_FINE_LOCATION) != PackageManager.PERMISSION_GRANTED) {
      permissions.add(Manifest.permission.ACCESS_FINE_LOCATION)
    }
    if (ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_COARSE_LOCATION) != PackageManager.PERMISSION_GRANTED) {
      permissions.add(Manifest.permission.ACCESS_COARSE_LOCATION)
    }
    if (permissions.isNotEmpty()) {
      suppressAutoPipUntilMs = SystemClock.elapsedRealtime() + 5000L
      ActivityCompat.requestPermissions(this, permissions.toTypedArray(), 1001)
    }
  }
}
