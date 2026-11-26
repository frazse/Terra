package dev.frazse.terra

import android.annotation.SuppressLint
import android.app.Presentation
import android.content.Context
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.view.Display
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.ConsoleMessage
import android.webkit.JavascriptInterface
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class DualScreenPresentation(
  outerContext: Context,
  display: Display,
  private val onAction: (String) -> Unit,
  private val onPageReady: () -> Unit
) : Presentation(outerContext, display) {

  private var webView: WebView? = null
  private var pageReady = false
  private var pendingPayload: String? = null
  private var pendingStateJson: String? = null

  @SuppressLint("SetJavaScriptEnabled")
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    setCancelable(false)
    setCanceledOnTouchOutside(false)

    window?.apply {
      addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
      // Keep primary WebView / MainActivity focus unaffected by taps on Screen 2 unless needed,
      // while still receiving touch/scroll events on Display 2.
      addFlags(WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE)
      setBackgroundDrawableResource(android.R.color.black)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
        setDecorFitsSystemWindows(false)
      }
    }

    val container = FrameLayout(context).apply {
      setBackgroundColor(Color.parseColor("#0b0e14"))
      layoutParams = ViewGroup.LayoutParams(
        ViewGroup.LayoutParams.MATCH_PARENT,
        ViewGroup.LayoutParams.MATCH_PARENT
      )
    }

    val wv = WebView(context).apply {
      layoutParams = FrameLayout.LayoutParams(
        FrameLayout.LayoutParams.MATCH_PARENT,
        FrameLayout.LayoutParams.MATCH_PARENT
      )
      setBackgroundColor(Color.parseColor("#0b0e14"))
      settings.apply {
        javaScriptEnabled = true
        domStorageEnabled = true
        allowFileAccess = true
        useWideViewPort = false
        loadWithOverviewMode = false
        mixedContentMode = WebSettings.MIXED_CONTENT_COMPATIBILITY_MODE
      }
      addJavascriptInterface(DualBridge(), "TerraDual")
      webChromeClient = object : WebChromeClient() {
        override fun onConsoleMessage(consoleMessage: ConsoleMessage): Boolean {
          if (consoleMessage.messageLevel() == ConsoleMessage.MessageLevel.ERROR ||
            consoleMessage.messageLevel() == ConsoleMessage.MessageLevel.WARNING
          ) {
            android.util.Log.w(
              "TerraDual",
              "[${consoleMessage.messageLevel()}] ${consoleMessage.message()} (${consoleMessage.sourceId()}:${consoleMessage.lineNumber()})"
            )
          }
          return true
        }
      }
      webViewClient = object : WebViewClient() {
        override fun shouldInterceptRequest(
          view: WebView,
          request: WebResourceRequest
        ): WebResourceResponse? {
          CssCompat.interceptCssIfNeeded(context, request)?.let { return it }
          return super.shouldInterceptRequest(view, request)
        }

        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
          return true
        }

        override fun onPageFinished(view: WebView, url: String) {
          super.onPageFinished(view, url)
          markReady()
        }
      }
    }

    container.addView(wv)
    setContentView(container)
    webView = wv

    hideSystemBars()

    val html = try {
      context.assets.open("dualscreen.html").bufferedReader().use { it.readText() }
    } catch (e: Exception) {
      "<html><body style='background:#0b0e14;color:#fff;font-family:sans-serif;padding:16px'>Failed to load dualscreen.html: ${e.message}</body></html>"
    }

    wv.loadDataWithBaseURL("https://biketerra.com/", html, "text/html", "UTF-8", null)
  }

  private fun hideSystemBars() {
    val win = window ?: return
    try {
      val controller = WindowCompat.getInsetsController(win, win.decorView)
      controller.hide(WindowInsetsCompat.Type.systemBars())
      controller.systemBarsBehavior =
        WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
    } catch (_: Exception) {}
  }

  private fun markReady() {
    if (pageReady) return
    pageReady = true
    pendingStateJson?.let { evalState(it) }
    pendingPayload?.let { evalPayload(it) }
    onPageReady()
  }

  fun updateState(stateJson: String) {
    pendingStateJson = stateJson
    if (!pageReady) return
    evalState(stateJson)
  }

  fun pushPayload(payloadJson: String) {
    pendingPayload = payloadJson
    if (!pageReady) return
    evalPayload(payloadJson)
  }

  private fun evalState(stateJson: String) {
    val wv = webView ?: return
    val quoted = org.json.JSONObject.quote(stateJson)
    wv.post {
      try {
        wv.evaluateJavascript(
          "window.__terraDualSetState && window.__terraDualSetState(JSON.parse($quoted));",
          null
        )
      } catch (_: Exception) {}
    }
  }

  private fun evalPayload(payloadJson: String) {
    val wv = webView ?: return
    val quoted = org.json.JSONObject.quote(payloadJson)
    wv.post {
      try {
        wv.evaluateJavascript(
          "window.__terraDualReceive && window.__terraDualReceive($quoted);",
          null
        )
      } catch (_: Exception) {}
    }
  }

  override fun onStop() {
    super.onStop()
    webView?.let { wv ->
      try {
        (wv.parent as? ViewGroup)?.removeView(wv)
        wv.stopLoading()
        wv.destroy()
      } catch (_: Exception) {}
    }
    webView = null
    pageReady = false
  }

  private inner class DualBridge {
    @JavascriptInterface
    fun dispatchAction(json: String) {
      onAction(json)
    }

    @JavascriptInterface
    fun setFocusable(focusable: Boolean) {
      val win = window ?: return
      webView?.post {
        try {
          if (focusable) {
            win.clearFlags(WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE)
          } else {
            win.addFlags(WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE)
          }
        } catch (_: Exception) {}
      }
    }

    @JavascriptInterface
    fun onReady() {
      webView?.post { markReady() }
    }
  }
}
