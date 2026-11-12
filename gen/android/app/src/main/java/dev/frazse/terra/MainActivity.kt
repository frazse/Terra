package dev.frazse.terra

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.CookieManager
import android.webkit.WebChromeClient
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import java.io.File
import java.io.FileOutputStream

class MainActivity : TauriActivity() {
  companion object {
    private var instance: MainActivity? = null

    @JvmStatic
    fun setFullscreenJni(enable: Boolean) {
      instance?.runOnUiThread {
        instance?.toggleFullscreen(enable)
      }
    }
  }

  private var isFullscreen = true

  override fun onCreate(savedInstanceState: Bundle?) {
    instance = this
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

    val cookieManager = CookieManager.getInstance()
    cookieManager.setAcceptCookie(true)

    window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
      setShowWhenLocked(true)
      setTurnScreenOn(true)
    }

    extractBundledAddons()

    ViewCompat.setOnApplyWindowInsetsListener(window.decorView) { view, insets ->
      if (!isFullscreen) {
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
    }
  }

  override fun onPause() {
    super.onPause()
    try {
      CookieManager.getInstance().flush()
    } catch (_: Exception) {}
  }

  override fun onDestroy() {
    try {
      CookieManager.getInstance().flush()
    } catch (_: Exception) {}
    super.onDestroy()
  }

  private fun extractBundledAddons() {
    try {
      val extDir = getExternalFilesDir(null) ?: return
      val addonsDir = File(extDir, "addons")
      if (!addonsDir.exists()) addonsDir.mkdirs()

      // keep in sync with BUNDLED_ADDONS in build-apk.sh
      val bundledAddons = listOf("zwift-ride", "garmin-hud", "fps-monitor")
      for (addonName in bundledAddons) {
        val targetDir = File(addonsDir, addonName)
        if (!shouldExtract(addonName, targetDir)) continue
        if (targetDir.exists()) targetDir.deleteRecursively() // stale copy: replace it
        targetDir.mkdirs()
        val files = assets.list("addons/$addonName") ?: continue
        for (f in files) {
          assets.open("addons/$addonName/$f").use { inStream ->
            FileOutputStream(File(targetDir, f)).use { outStream ->
              inStream.copyTo(outStream)
            }
          }
        }
        android.util.Log.i(
          "Terra",
          "Extracted bundled $addonName addon v${bundledAddonVersion(addonName) ?: "?"}"
        )
      }
    } catch (e: Exception) {
      android.util.Log.e("Terra", "Error extracting bundled addons", e)
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

  fun toggleFullscreen(enable: Boolean? = null) {
    val target = enable ?: !isFullscreen
    isFullscreen = target
    val insetsController = WindowCompat.getInsetsController(window, window.decorView)
    if (target) {
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

  private fun setupWebViewFullscreen() {
    val wv = findWebView(window.decorView) ?: return
    wv.keepScreenOn = true
    try {
      CookieManager.getInstance().setAcceptThirdPartyCookies(wv, true)
    } catch (_: Exception) {}
    wv.webChromeClient = object : WebChromeClient() {
      override fun onShowCustomView(view: View?, callback: CustomViewCallback?) {
        toggleFullscreen(true)
      }

      override fun onHideCustomView() {
        toggleFullscreen(false)
      }
    }
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
    if (ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_FINE_LOCATION) != PackageManager.PERMISSION_GRANTED) {
      permissions.add(Manifest.permission.ACCESS_FINE_LOCATION)
    }
    if (ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_COARSE_LOCATION) != PackageManager.PERMISSION_GRANTED) {
      permissions.add(Manifest.permission.ACCESS_COARSE_LOCATION)
    }
    if (permissions.isNotEmpty()) {
      ActivityCompat.requestPermissions(this, permissions.toTypedArray(), 1001)
    }
  }
}
