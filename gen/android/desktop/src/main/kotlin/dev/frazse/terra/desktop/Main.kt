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
import org.cef.handler.CefLoadHandlerAdapter
import java.io.File

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
                    val rootDir = File(System.getProperty("user.dir")).parentFile
                    val shimFile = File(rootDir, "shim/bluetooth-shim.js")
                    val fullInit = if (shimFile.exists()) {
                        val shim = shimFile.readText()
                        val loader = File(rootDir, "shim/addons-loader.js").readText()
                        val ui = File(rootDir, "shim/terra-ui.js").readText()
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

                    if (fullInit.isNotEmpty()) {
                        client.addLoadHandler(object : CefLoadHandlerAdapter() {
                            override fun onLoadEnd(b: CefBrowser?, frame: CefFrame?, httpStatusCode: Int) {
                                if (frame != null && frame.isMain) {
                                    frame.executeJavaScript(fullInit, frame.url, 0)
                                }
                            }
                        })
                    }

                    val cefBrowser = client.createBrowser("https://biketerra.com", false, false)
                    browser = cefBrowser
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
