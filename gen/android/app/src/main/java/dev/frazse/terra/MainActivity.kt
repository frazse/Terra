package dev.frazse.terra

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.Settings
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.CookieManager
import android.webkit.WebChromeClient
import android.webkit.WebView
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
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream

class MainActivity : TauriActivity() {
  companion object {
    private var instance: MainActivity? = null
    private var dataDirResult: String = "pending" // "pending" | "cancelled" | "success:<path>" | "error:<msg>"

    @Keep
    @JvmStatic
    fun setFullscreenJni(enable: Boolean) {
      instance?.runOnUiThread {
        instance?.toggleFullscreen(enable)
      }
    }

    @Keep
    @JvmStatic
    fun startDataDirPicker() {
      dataDirResult = "pending"
      instance?.runOnUiThread {
        val act = instance ?: return@runOnUiThread
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
  }

  private var isFullscreen = true
  private var dataDirLauncher: ActivityResultLauncher<Uri?>? = null
  private var storagePermLauncher: ActivityResultLauncher<Intent>? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    instance = this
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

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
      val dataDir = getTerraDataDir()
      val addonsDir = File(dataDir, "addons")
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
