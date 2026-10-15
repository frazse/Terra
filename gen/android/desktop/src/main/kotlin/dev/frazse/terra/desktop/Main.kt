package dev.frazse.terra.desktop

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.MaterialTheme
import androidx.compose.material.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.awt.SwingPanel
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import me.friwi.jcefmaven.CefAppBuilder
import org.cef.browser.CefBrowser
import org.cef.browser.CefFrame
import org.cef.browser.CefMessageRouter
import org.cef.callback.CefQueryCallback
import org.cef.handler.CefLoadHandlerAdapter
import org.cef.handler.CefMessageRouterHandlerAdapter
import org.cef.network.CefRequest
import java.io.File
import java.util.concurrent.ConcurrentHashMap

fun main() = application {
    Window(
        onCloseRequest = ::exitApplication,
        title = "Terra (Compose Desktop)"
    ) {
        MaterialTheme {
            var browser by remember { mutableStateOf<CefBrowser?>(null) }
            var statusMessage by remember { mutableStateOf("Initializing Chromium Engine (CEF)...") }

            LaunchedEffect(Unit) {
                try {
                    LinuxBleManager.init()

                    var rootDir: File? = File(System.getProperty("user.dir"))
                    while (rootDir != null && !File(rootDir, "shim/bluetooth-shim.js").exists()) {
                        rootDir = rootDir.parentFile
                    }
                    val projRoot = rootDir ?: File(System.getProperty("user.dir"))

                    val shimFile = File(projRoot, "shim/bluetooth-shim.js")
                    println("Shim file path: ${shimFile.absolutePath}, exists: ${shimFile.exists()}")

                    val fullInit = if (shimFile.exists()) {
                        val shim = shimFile.readText()
                        val loader = File(projRoot, "shim/addons-loader.js").readText()
                        val ui = File(projRoot, "shim/terra-ui.js").readText()
                        "$shim\n;\n$loader\n;\n$ui"
                    } else ""

                    val cacheDir = File(System.getProperty("user.home"), ".local/share/dev.frazse.terra/cef-cache")
                    if (!cacheDir.exists()) cacheDir.mkdirs()

                    val builder = CefAppBuilder()
                    builder.setInstallDir(File(System.getProperty("user.home"), ".local/share/dev.frazse.terra/jcef"))
                    builder.cefSettings.windowless_rendering_enabled = false
                    builder.cefSettings.cache_path = cacheDir.absolutePath
                    builder.cefSettings.persist_session_cookies = true

                    builder.addJcefArgs("--force-device-scale-factor=1.25")
                    builder.addJcefArgs("--enable-high-dpi-support=1")

                    val cefApp = builder.build()
                    val client = cefApp.createClient()

                    val pendingRequestCallbacks = ConcurrentHashMap<Long, CefQueryCallback>()

                    val msgRouter = CefMessageRouter.create()
                    msgRouter.addHandler(object : CefMessageRouterHandlerAdapter() {
                        override fun onQuery(
                            b: CefBrowser?,
                            frame: CefFrame?,
                            queryId: Long,
                            request: String?,
                            persistent: Boolean,
                            callback: CefQueryCallback?
                        ): Boolean {
                            if (request == null || callback == null) return false
                            try {
                                val jsonReq = if (request.trim().startsWith("{")) org.json.JSONObject(request) else null
                                val cmd = jsonReq?.optString("cmd") ?: request
                                val args = jsonReq?.optJSONObject("args")
                                println("JCEF Query: cmd=$cmd, args=$args")

                                when (cmd) {
                                    "ble_start_scan" -> {
                                        LinuxBleManager.startScan()
                                        callback.success("OK")
                                    }
                                    "chooser_candidates" -> {
                                        callback.success(LinuxBleManager.getDiscoveredDevicesJson())
                                    }
                                    "ble_stop_scan", "chooser_cancel" -> {
                                        LinuxBleManager.stopScan()
                                        callback.success("OK")
                                    }
                                    "ble_get_devices" -> {
                                        callback.success(LinuxBleManager.getDiscoveredDevicesJson())
                                    }
                                    "ble_request_device" -> {
                                        val silent = args?.optBoolean("silent", false) ?: false
                                        if (!silent) {
                                            b?.executeJavaScript("window.showChooserModal()", b.url, 0)
                                        }
                                        pendingRequestCallbacks[queryId] = callback

                                        Thread {
                                            val reqRes = LinuxBleManager.sendRpcPublic("ble_request_device", args ?: org.json.JSONObject())
                                            println("Rust ble_request_device response: $reqRes")
                                            if (reqRes.trim().startsWith("{")) {
                                                val devJson = org.json.JSONObject(reqRes)
                                                val devId = devJson.optString("id")
                                                val devName = devJson.optString("name")
                                                if (devId.isNotBlank()) {
                                                    val resObj = org.json.JSONObject()
                                                    resObj.put("id", devId)
                                                    resObj.put("name", if (devName.isNotBlank()) devName else devId)

                                                    val cb = pendingRequestCallbacks.remove(queryId)
                                                    cb?.success(resObj.toString())
                                                } else {
                                                    val cb = pendingRequestCallbacks.remove(queryId)
                                                    cb?.failure(404, "No device found")
                                                }
                                            } else {
                                                val cb = pendingRequestCallbacks.remove(queryId)
                                                cb?.failure(404, "No device found")
                                            }
                                        }.start()
                                    }
                                    "chooser_pick" -> {
                                        val pickedId = args?.optString("id") ?: ""
                                        LinuxBleManager.sendRpcPublic("chooser_pick", args ?: org.json.JSONObject())

                                        val candidatesJson = LinuxBleManager.getDiscoveredDevicesJson()
                                        var devName = pickedId
                                        try {
                                            val arr = org.json.JSONArray(candidatesJson)
                                            for (i in 0 until arr.length()) {
                                                val c = arr.getJSONObject(i)
                                                if (c.optString("id") == pickedId) {
                                                    val n = c.optString("name")
                                                    if (n.isNotBlank()) devName = n
                                                    break
                                                }
                                            }
                                        } catch (_: Exception) {}

                                        val devObj = org.json.JSONObject()
                                        devObj.put("id", pickedId)
                                        devObj.put("name", devName)

                                        for ((qid, cb) in pendingRequestCallbacks) {
                                            cb.success(devObj.toString())
                                        }
                                        pendingRequestCallbacks.clear()
                                        callback.success("OK")
                                    }
                                    "ble_connect" -> {
                                        val addr = args?.optString("id") ?: ""
                                        val servicesJson = LinuxBleManager.connect(addr)
                                        callback.success(servicesJson)
                                    }
                                    "ble_read" -> {
                                        val addr = args?.optString("id") ?: ""
                                        val service = args?.optString("service") ?: ""
                                        val charac = args?.optString("characteristic") ?: ""
                                        val res = LinuxBleManager.readCharacteristic(addr, service, charac)
                                        callback.success(res)
                                    }
                                    "ble_write" -> {
                                        val addr = args?.optString("id") ?: ""
                                        val service = args?.optString("service") ?: ""
                                        val charac = args?.optString("characteristic") ?: ""
                                        val valB64 = args?.optString("value") ?: ""
                                        val withResp = args?.optBoolean("withResponse", true) ?: true
                                        val res = LinuxBleManager.writeCharacteristic(addr, service, charac, valB64, withResp)
                                        callback.success(res)
                                    }
                                    "ble_subscribe" -> {
                                        val addr = args?.optString("id") ?: ""
                                        val service = args?.optString("service") ?: ""
                                        val charac = args?.optString("characteristic") ?: ""
                                        val res = LinuxBleManager.subscribeCharacteristic(addr, service, charac, true)
                                        callback.success(res)
                                    }
                                    "ble_unsubscribe" -> {
                                        val addr = args?.optString("id") ?: ""
                                        val service = args?.optString("service") ?: ""
                                        val charac = args?.optString("characteristic") ?: ""
                                        val res = LinuxBleManager.subscribeCharacteristic(addr, service, charac, false)
                                        callback.success(res)
                                    }
                                    "ble_disconnect", "ble_forget" -> {
                                        val addr = args?.optString("id") ?: ""
                                        LinuxBleManager.disconnect(addr)
                                        callback.success("OK")
                                    }
                                    else -> {
                                        callback.success("{}")
                                    }
                                }
                                return true
                            } catch (e: Exception) {
                                println("Error handling JCEF Query: ${e.message}")
                                callback.failure(500, e.message ?: "Internal Error")
                                return true
                            }
                        }
                    }, true)
                    client.addMessageRouter(msgRouter)

                    if (fullInit.isNotEmpty()) {
                        client.addLoadHandler(object : CefLoadHandlerAdapter() {
                            override fun onLoadStart(b: CefBrowser?, frame: CefFrame?, transitionType: CefRequest.TransitionType?) {
                                if (frame != null && frame.isMain) {
                                    frame.executeJavaScript(fullInit, frame.url, 0)
                                }
                            }

                            override fun onLoadEnd(b: CefBrowser?, frame: CefFrame?, httpStatusCode: Int) {
                                if (frame != null && frame.isMain) {
                                    frame.executeJavaScript(fullInit, frame.url, 0)
                                }
                            }
                        })
                    }

                    val cefBrowser = client.createBrowser("https://biketerra.com", false, false)
                    browser = cefBrowser
                    LinuxBleManager.onJsEval = { code ->
                        cefBrowser.executeJavaScript(code, cefBrowser.url, 0)
                    }
                } catch (e: Exception) {
                    statusMessage = "Error initializing Chromium: " + e.message
                    e.printStackTrace()
                }
            }

            val currentBrowser = browser
            if (currentBrowser != null) {
                SwingPanel(
                    modifier = Modifier.fillMaxSize(),
                    factory = { currentBrowser.uiComponent }
                )
            } else {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(statusMessage)
                }
            }
        }
    }
}
