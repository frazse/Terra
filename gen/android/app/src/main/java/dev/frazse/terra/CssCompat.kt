package dev.frazse.terra

import android.content.Context
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import androidx.webkit.WebViewCompat
import java.io.ByteArrayInputStream
import java.net.HttpURLConnection
import java.net.URI
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.roundToInt
import kotlin.math.sin

/**
 * Compatibility layer for older Android System WebViews (< Chromium 111) such as
 * Chromium 109 on Android 13 devices where OEM firmware prevents updating WebView.
 *
 * Chromium 111 introduced both `oklch()`/`oklab()` and `color-mix()`. Without this
 * translation, modern Biketerra CSS tokens (`--sunk`, `--ground`, `--surface`, `--ink`,
 * `--success`, `--danger`, `--selected`, etc.) fail to parse, leaving UI surfaces,
 * cards, buttons, and borders unstyled.
 */
object CssCompat {
  private const val MIN_NATIVE_CHROMIUM_MAJOR = 111

  data class CssColor(val r: Int, val g: Int, val b: Int, val a: Double = 1.0) {
    fun toRgbTriplet(): String = "$r, $g, $b"

    fun toCssString(): String {
      return if (a >= 0.9995) {
        "rgb($r, $g, $b)"
      } else if (a <= 0.0005) {
        "rgba(0, 0, 0, 0)"
      } else {
        "rgba($r, $g, $b, ${formatAlpha(a)})"
      }
    }
  }

  @Volatile
  private var cachedMajorVersion: Int? = null

  private val patchedCssCache = ConcurrentHashMap<String, ByteArray>()

  // Pre-seeded Biketerra :root palette (converted from 0.B3sFB2Q7.css) so lazy route
  // stylesheets (e.g. ride.*.css) always resolve :root tokens regardless of load order.
  // Dynamically refreshed whenever any CSS defining :root tokens is transformed.
  private val rootVars = ConcurrentHashMap<String, CssColor>().apply {
    putAll(defaultBiketerraTokens())
  }

  private val colorVarNames = ConcurrentHashMap.newKeySet<String>().apply {
    addAll(rootVars.keys)
    add("--tone")
    add("--chip")
    add("--glow")
    add("--horizon")
    add("--dim")
    add("--mid")
  }

  private fun defaultBiketerraTokens(): Map<String, CssColor> {
    val map = LinkedHashMap<String, CssColor>()
    fun hex(name: String, h: String) {
      parseHexColor(h)?.let { map[name] = it }
    }
    fun oklch(name: String, l: Double, c: Double, h: Double) {
      map[name] = oklchToCssColor(l, c, h, 1.0)
    }

    hex("--bt-e", "#572667")
    hex("--bt-d", "#7f347c")
    hex("--bt-c", "#b14674")
    hex("--bt-b", "#dc5666")
    hex("--bt-a", "#ff6262")
    hex("--bt-9", "#fc8174")
    hex("--bt-8", "#ffa574")
    hex("--bt-7", "#ffcd7c")
    hex("--bt-6", "#ffe37d")

    oklch("--sunk", 0.255, 0.03, 318.0)
    oklch("--ground", 0.29, 0.032, 318.0)
    oklch("--surface", 0.34, 0.036, 318.0)
    oklch("--raised", 0.38, 0.036, 318.0)
    oklch("--line", 0.42, 0.034, 318.0)
    oklch("--ink", 0.93, 0.014, 70.0)
    oklch("--ink-2", 0.80, 0.022, 340.0)
    oklch("--ink-3", 0.72, 0.028, 335.0)
    oklch("--success", 0.80, 0.16, 150.0)
    oklch("--danger", 0.74, 0.15, 25.0)
    oklch("--caution", 0.80, 0.14, 62.0)
    oklch("--type-casual", 0.80, 0.12, 160.0)
    oklch("--type-race", 0.78, 0.13, 25.0)
    oklch("--type-tt", 0.79, 0.10, 255.0)

    map["--bt-8"]?.let { map["--link"] = it }
    map["--bt-7"]?.let {
      map["--focus"] = it
      map["--warn"] = it
      map["--type-workout"] = it
    }
    return map
  }

  fun getWebViewMajorVersion(context: Context): Int {
    cachedMajorVersion?.let { return it }
    val major = try {
      val pkg = WebViewCompat.getCurrentWebViewPackage(context.applicationContext)
      val ver = pkg?.versionName ?: ""
      ver.substringBefore('.').trim().toIntOrNull() ?: -1
    } catch (_: Exception) {
      -1
    }
    cachedMajorVersion = major
    return major
  }

  fun needsColorCompat(context: Context): Boolean {
    val major = getWebViewMajorVersion(context)
    return major in 1 until MIN_NATIVE_CHROMIUM_MAJOR
  }

  fun interceptCssIfNeeded(
    context: Context,
    request: WebResourceRequest
  ): WebResourceResponse? {
    if (!needsColorCompat(context)) return null
    if (!"GET".equals(request.method, ignoreCase = true)) return null
    val uri = request.url ?: return null
    if (!"https".equals(uri.scheme, ignoreCase = true)) return null
    val host = uri.host?.lowercase(Locale.US) ?: return null
    if (host != "biketerra.com" && host != "www.biketerra.com" && !host.endsWith(".biketerra.com")) {
      return null
    }
    val path = uri.path ?: return null
    if (!path.endsWith(".css", ignoreCase = true)) return null

    val urlStr = uri.toString()
    patchedCssCache[urlStr]?.let { cached ->
      return cssResponse(cached, request, cacheControl = "no-store")
    }

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
      android.util.Log.w("TerraCss", "CSS fetch failed for $urlStr: ${e.message}")
      return null
    } finally {
      conn?.disconnect()
    }

    val src = String(rawBytes, Charsets.UTF_8)
    val transformed = transformCss(src)
    val outBytes = transformed.toByteArray(Charsets.UTF_8)
    patchedCssCache[urlStr] = outBytes
    if (transformed != src) {
      android.util.Log.i(
        "TerraCss",
        "Patched ${path.substringAfterLast('/')} for WebView ${getWebViewMajorVersion(context)} (${rawBytes.size} -> ${outBytes.size} B)"
      )
    }
    return cssResponse(outBytes, request, cacheControl = "no-store")
  }

  private fun cssResponse(
    bytes: ByteArray,
    request: WebResourceRequest,
    cacheControl: String
  ): WebResourceResponse {
    val origin = request.requestHeaders?.entries
      ?.firstOrNull { it.key.equals("origin", ignoreCase = true) }
      ?.value
    val headers = mutableMapOf(
      "Content-Type" to "text/css; charset=utf-8",
      "Cache-Control" to cacheControl,
      "Access-Control-Allow-Origin" to (origin ?: "https://biketerra.com"),
      "Vary" to "Origin"
    )
    if (origin != null) {
      headers["Access-Control-Allow-Credentials"] = "true"
    }
    return WebResourceResponse(
      "text/css",
      "utf-8",
      200,
      "OK",
      headers,
      ByteArrayInputStream(bytes)
    )
  }

  fun transformCss(css: String): String {
    if (css.isEmpty()) return css
    // Step 1: Convert oklch(...) and oklab(...) literals to rgb(...) / rgba(...)
    var out = replaceColorFunctions(css)
    // Step 2: Parse :root declarations so color-mix() uses live tokens
    updateRootVarsFromCss(out)
    // Step 3: Resolve color-mix(...) expressions
    out = replaceColorMix(out)
    // Step 4: Re-parse :root in case :root itself contained color-mix() (e.g. --selected)
    updateRootVarsFromCss(out)
    // Step 5: Emit --<var>-rgb companion variables for dynamic var() in color-mix (e.g. --tone, --chip, --glow)
    out = injectRgbCompanionVars(out)
    return out
  }

  private fun findMatchingParen(s: String, openParenIdx: Int): Int {
    var depth = 0
    var i = openParenIdx
    var inSingle = false
    var inDouble = false
    while (i < s.length) {
      val ch = s[i]
      if (ch == '\\' && i + 1 < s.length) {
        i += 2
        continue
      }
      if (!inDouble && ch == '\'') {
        inSingle = !inSingle
      } else if (!inSingle && ch == '"') {
        inDouble = !inDouble
      } else if (!inSingle && !inDouble) {
        if (ch == '(') depth++
        else if (ch == ')') {
          depth--
          if (depth == 0) return i
        }
      }
      i++
    }
    return -1
  }

  private fun replaceColorFunctions(css: String): String {
    val fnRegex = Regex("""(?<![\w-])(oklch|oklab)\(""", RegexOption.IGNORE_CASE)
    val sb = StringBuilder(css.length)
    var pos = 0
    while (pos < css.length) {
      val match = fnRegex.find(css, pos)
      if (match == null) {
        sb.append(css, pos, css.length)
        break
      }
      val start = match.range.first
      val openIdx = match.range.last
      val closeIdx = findMatchingParen(css, openIdx)
      if (closeIdx < 0) {
        sb.append(css, pos, css.length)
        break
      }
      sb.append(css, pos, start)
      val fnName = match.groupValues[1].lowercase(Locale.US)
      val inner = css.substring(openIdx + 1, closeIdx)
      val converted = if (fnName == "oklch") parseOklchArgs(inner) else parseOklabArgs(inner)
      if (converted != null) {
        sb.append(converted.toCssString())
      } else {
        sb.append(css, start, closeIdx + 1)
      }
      pos = closeIdx + 1
    }
    return sb.toString()
  }

  private fun parseOklchArgs(inner: String): CssColor? {
    val parts = splitTopLevelSlash(inner) ?: return null
    val tokens = splitWhitespaceTokens(parts.first)
    if (tokens.size != 3) return null
    val l = parsePercentOrNumber(tokens[0], 1.0)?.coerceIn(0.0, 1.0) ?: return null
    val c = parsePercentOrNumber(tokens[1], 0.4)?.coerceAtLeast(0.0) ?: return null
    val h = parseHueDegrees(tokens[2]) ?: return null
    val a = if (parts.second != null) {
      (parsePercentOrNumber(parts.second!!, 1.0) ?: return null).coerceIn(0.0, 1.0)
    } else {
      1.0
    }
    return oklchToCssColor(l, c, h, a)
  }

  private fun parseOklabArgs(inner: String): CssColor? {
    val parts = splitTopLevelSlash(inner) ?: return null
    val tokens = splitWhitespaceTokens(parts.first)
    if (tokens.size != 3) return null
    val l = parsePercentOrNumber(tokens[0], 1.0)?.coerceIn(0.0, 1.0) ?: return null
    val aCoord = parsePercentOrNumber(tokens[1], 0.4) ?: return null
    val bCoord = parsePercentOrNumber(tokens[2], 0.4) ?: return null
    val alpha = if (parts.second != null) {
      (parsePercentOrNumber(parts.second!!, 1.0) ?: return null).coerceIn(0.0, 1.0)
    } else {
      1.0
    }
    return oklabToCssColor(l, aCoord, bCoord, alpha)
  }

  private fun splitTopLevelSlash(s: String): Pair<String, String?>? {
    var depth = 0
    var slashIdx = -1
    for (i in s.indices) {
      when (s[i]) {
        '(' -> depth++
        ')' -> depth--
        '/' -> if (depth == 0) {
          if (slashIdx >= 0) return null
          slashIdx = i
        }
      }
    }
    return if (slashIdx < 0) {
      Pair(s.trim(), null)
    } else {
      Pair(s.substring(0, slashIdx).trim(), s.substring(slashIdx + 1).trim())
    }
  }

  private fun splitWhitespaceTokens(s: String): List<String> {
    val out = ArrayList<String>(4)
    var i = 0
    while (i < s.length) {
      while (i < s.length && s[i].isWhitespace()) i++
      if (i >= s.length) break
      val start = i
      var depth = 0
      while (i < s.length) {
        val ch = s[i]
        if (ch == '(') depth++
        else if (ch == ')') depth--
        else if (depth == 0 && ch.isWhitespace()) break
        i++
      }
      out.add(s.substring(start, i))
    }
    return out
  }

  private fun parsePercentOrNumber(token: String, percentScale: Double): Double? {
    val t = token.trim().lowercase(Locale.US)
    if (t == "none") return 0.0
    return if (t.endsWith("%")) {
      t.dropLast(1).toDoubleOrNull()?.let { (it / 100.0) * percentScale }
    } else {
      t.toDoubleOrNull()
    }
  }

  private fun parseHueDegrees(token: String): Double? {
    val t = token.trim().lowercase(Locale.US)
    return when {
      t == "none" -> 0.0
      t.endsWith("deg") -> t.dropLast(3).toDoubleOrNull()
      t.endsWith("rad") -> t.dropLast(3).toDoubleOrNull()?.let { it * 180.0 / PI }
      t.endsWith("grad") -> t.dropLast(4).toDoubleOrNull()?.let { it * 0.9 }
      t.endsWith("turn") -> t.dropLast(4).toDoubleOrNull()?.let { it * 360.0 }
      else -> t.toDoubleOrNull()
    }
  }

  private fun oklabToLinearSrgb(l: Double, a: Double, b: Double): Triple<Double, Double, Double> {
    val l_ = l + 0.3963377774 * a + 0.2158037573 * b
    val m_ = l - 0.1055613458 * a - 0.0638541728 * b
    val s_ = l - 0.0894841775 * a - 1.2914855480 * b

    val l3 = l_ * l_ * l_
    val m3 = m_ * m_ * m_
    val s3 = s_ * s_ * s_

    val rLin = +4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3
    val gLin = -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3
    val bLin = -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3
    return Triple(rLin, gLin, bLin)
  }

  private fun linearToSrgbByte(lin: Double): Int {
    val clamped = lin.coerceIn(0.0, 1.0)
    val srgb = if (clamped <= 0.0031308) {
      12.92 * clamped
    } else {
      1.055 * clamped.pow(1.0 / 2.4) - 0.055
    }
    return (srgb * 255.0).roundToInt().coerceIn(0, 255)
  }

  private fun srgbByteToLinear(v: Int): Double {
    val c = v.coerceIn(0, 255) / 255.0
    return if (c <= 0.04045) c / 12.92 else ((c + 0.055) / 1.055).pow(2.4)
  }

  private fun cssColorToOklab(c: CssColor): Triple<Double, Double, Double> {
    val r = srgbByteToLinear(c.r)
    val g = srgbByteToLinear(c.g)
    val b = srgbByteToLinear(c.b)

    val l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b
    val m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b
    val s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b

    val l_ = Math.cbrt(l)
    val m_ = Math.cbrt(m)
    val s_ = Math.cbrt(s)

    val outL = 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_
    val outA = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_
    val outB = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_
    return Triple(outL, outA, outB)
  }

  private fun inSrgbGamut(r: Double, g: Double, b: Double): Boolean {
    return r in -1e-4..(1.0 + 1e-4) && g in -1e-4..(1.0 + 1e-4) && b in -1e-4..(1.0 + 1e-4)
  }

  fun oklchToCssColor(l: Double, c: Double, hDeg: Double, alpha: Double): CssColor {
    if (l <= 0.0) return CssColor(0, 0, 0, alpha)
    if (l >= 1.0) return CssColor(255, 255, 255, alpha)
    val rad = Math.toRadians(hDeg)
    val cosH = cos(rad)
    val sinH = sin(rad)
    var (rLin, gLin, bLin) = oklabToLinearSrgb(l, c * cosH, c * sinH)
    if (!inSrgbGamut(rLin, gLin, bLin) && c > 0.0) {
      var lo = 0.0
      var hi = c
      for (iter in 0 until 16) {
        val mid = 0.5 * (lo + hi)
        val (mr, mg, mb) = oklabToLinearSrgb(l, mid * cosH, mid * sinH)
        if (inSrgbGamut(mr, mg, mb)) {
          lo = mid
          rLin = mr
          gLin = mg
          bLin = mb
        } else {
          hi = mid
        }
      }
    }
    return CssColor(
      linearToSrgbByte(rLin),
      linearToSrgbByte(gLin),
      linearToSrgbByte(bLin),
      alpha.coerceIn(0.0, 1.0)
    )
  }

  fun oklabToCssColor(l: Double, aCoord: Double, bCoord: Double, alpha: Double): CssColor {
    val c = kotlin.math.hypot(aCoord, bCoord)
    val hDeg = if (c < 1e-8) 0.0 else Math.toDegrees(kotlin.math.atan2(bCoord, aCoord))
    return oklchToCssColor(l, c, hDeg, alpha)
  }

  private fun updateRootVarsFromCss(css: String) {
    val rootRegex = Regex(""":root\s*\{""")
    val rawDecls = LinkedHashMap<String, String>()
    var pos = 0
    while (pos < css.length) {
      val m = rootRegex.find(css, pos) ?: break
      val openBrace = m.range.last
      val closeBrace = findMatchingBrace(css, openBrace)
      if (closeBrace < 0) break
      val body = css.substring(openBrace + 1, closeBrace)
      parseCustomPropertyDecls(body, rawDecls)
      pos = closeBrace + 1
    }
    if (rawDecls.isEmpty()) return

    val working = HashMap<String, CssColor>(rootVars)
    for (pass in 0 until 6) {
      var changed = false
      for ((name, rawVal) in rawDecls) {
        val parsed = parseCssColor(rawVal, working)
        if (parsed != null && working[name] != parsed) {
          working[name] = parsed
          changed = true
        }
      }
      if (!changed) break
    }
    rootVars.putAll(working)
    colorVarNames.addAll(working.keys)
  }

  private fun findMatchingBrace(s: String, openBraceIdx: Int): Int {
    var depth = 0
    var i = openBraceIdx
    var inSingle = false
    var inDouble = false
    while (i < s.length) {
      val ch = s[i]
      if (ch == '\\' && i + 1 < s.length) {
        i += 2
        continue
      }
      if (!inDouble && ch == '\'') inSingle = !inSingle
      else if (!inSingle && ch == '"') inDouble = !inDouble
      else if (!inSingle && !inDouble) {
        if (ch == '{') depth++
        else if (ch == '}') {
          depth--
          if (depth == 0) return i
        }
      }
      i++
    }
    return -1
  }

  private fun parseCustomPropertyDecls(body: String, out: MutableMap<String, String>) {
    val decls = splitTopLevelDeclarations(body)
    for ((_, _, name, value) in decls) {
      if (name.startsWith("--") && !name.endsWith("-rgb")) {
        out[name] = value
      }
    }
  }

  private data class DeclSpan(
    val declStart: Int,
    val valueEnd: Int,
    val name: String,
    val value: String
  )

  private fun splitTopLevelDeclarations(body: String): List<DeclSpan> {
    val res = ArrayList<DeclSpan>()
    var i = 0
    var declStart = 0
    var colonIdx = -1
    var parenDepth = 0
    var inSingle = false
    var inDouble = false
    while (i <= body.length) {
      val atEnd = i == body.length
      val ch = if (atEnd) ';' else body[i]
      if (!atEnd && ch == '\\' && i + 1 < body.length) {
        i += 2
        continue
      }
      if (!atEnd && !inDouble && ch == '\'') inSingle = !inSingle
      else if (!atEnd && !inSingle && ch == '"') inDouble = !inDouble
      else if (!atEnd && !inSingle && !inDouble) {
        if (ch == '(') parenDepth++
        else if (ch == ')') parenDepth--
        else if (ch == ':' && parenDepth == 0 && colonIdx < 0) colonIdx = i
      }
      if ((atEnd || (ch == ';' && parenDepth == 0 && !inSingle && !inDouble))) {
        if (colonIdx > declStart) {
          val name = body.substring(declStart, colonIdx).trim()
          val value = body.substring(colonIdx + 1, i).trim()
          if (name.isNotEmpty() && value.isNotEmpty()) {
            res.add(DeclSpan(declStart, i, name, value))
          }
        }
        declStart = i + 1
        colonIdx = -1
      }
      i++
    }
    return res
  }

  private fun replaceColorMix(css: String): String {
    val mixRegex = Regex("""(?<![\w-])color-mix\(""", RegexOption.IGNORE_CASE)
    val sb = StringBuilder(css.length)
    var pos = 0
    while (pos < css.length) {
      val match = mixRegex.find(css, pos)
      if (match == null) {
        sb.append(css, pos, css.length)
        break
      }
      val start = match.range.first
      val openIdx = match.range.last
      val closeIdx = findMatchingParen(css, openIdx)
      if (closeIdx < 0) {
        sb.append(css, pos, css.length)
        break
      }
      sb.append(css, pos, start)
      val innerRaw = css.substring(openIdx + 1, closeIdx)
      // Resolve any nested color-mix inside arguments first
      val inner = if (innerRaw.contains("color-mix(", ignoreCase = true)) {
        replaceColorMix(innerRaw)
      } else {
        innerRaw
      }
      val resolved = evalColorMix(inner)
      if (resolved != null) {
        sb.append(resolved)
      } else {
        sb.append(css, start, closeIdx + 1)
      }
      pos = closeIdx + 1
    }
    return sb.toString()
  }

  private data class MixPart(val colorExpr: String, val pct: Double?)

  private fun evalColorMix(inner: String): String? {
    val args = splitTopLevelCommas(inner)
    if (args.size != 3) return null
    val method = args[0].trim().lowercase(Locale.US)
    if (!method.startsWith("in ")) return null
    val isSrgb = method.contains("srgb") || method.contains("hsl")

    val part1 = parseMixPart(args[1]) ?: return null
    val part2 = parseMixPart(args[2]) ?: return null

    var p1 = part1.pct
    var p2 = part2.pct
    if (p1 == null && p2 == null) {
      p1 = 50.0
      p2 = 50.0
    } else if (p1 != null && p2 == null) {
      p2 = (100.0 - p1).coerceAtLeast(0.0)
    } else if (p1 == null && p2 != null) {
      p1 = (100.0 - p2).coerceAtLeast(0.0)
    }
    val v1 = p1!!
    val v2 = p2!!
    val sum = v1 + v2
    if (sum <= 1e-6) return "rgba(0, 0, 0, 0)"
    val alphaMult = if (sum < 100.0) (sum / 100.0).coerceIn(0.0, 1.0) else 1.0
    val w1 = v1 / sum
    val w2 = v2 / sum

    val e1 = part1.colorExpr.trim()
    val e2 = part2.colorExpr.trim()
    val isT1 = e1.equals("transparent", ignoreCase = true)
    val isT2 = e2.equals("transparent", ignoreCase = true)
    if (isT1 && isT2) return "rgba(0, 0, 0, 0)"

    val c1 = if (isT1) CssColor(0, 0, 0, 0.0) else parseCssColor(e1, rootVars)
    val c2 = if (isT2) CssColor(0, 0, 0, 0.0) else parseCssColor(e2, rootVars)

    // Dynamic custom property mixed with transparent (e.g. var(--tone), var(--chip), var(--glow))
    if (c1 == null && isT2) {
      extractSimpleVarName(e1)?.let { varName ->
        return buildDynamicRgba(varName, (w1 * alphaMult).coerceIn(0.0, 1.0))
      }
      return null
    }
    if (c2 == null && isT1) {
      extractSimpleVarName(e2)?.let { varName ->
        return buildDynamicRgba(varName, (w2 * alphaMult).coerceIn(0.0, 1.0))
      }
      return null
    }

    if (c1 == null || c2 == null) return null

    if (isT2) {
      return CssColor(c1.r, c1.g, c1.b, (c1.a * w1 * alphaMult).coerceIn(0.0, 1.0)).toCssString()
    }
    if (isT1) {
      return CssColor(c2.r, c2.g, c2.b, (c2.a * w2 * alphaMult).coerceIn(0.0, 1.0)).toCssString()
    }

    val a1 = c1.a.coerceIn(0.0, 1.0)
    val a2 = c2.a.coerceIn(0.0, 1.0)
    val pmAlpha = w1 * a1 + w2 * a2
    if (pmAlpha <= 1e-6) return "rgba(0, 0, 0, 0)"

    val outColor = if (isSrgb) {
      val r = ((w1 * a1 * c1.r + w2 * a2 * c2.r) / pmAlpha).roundToInt().coerceIn(0, 255)
      val g = ((w1 * a1 * c1.g + w2 * a2 * c2.g) / pmAlpha).roundToInt().coerceIn(0, 255)
      val b = ((w1 * a1 * c1.b + w2 * a2 * c2.b) / pmAlpha).roundToInt().coerceIn(0, 255)
      CssColor(r, g, b, (pmAlpha * alphaMult).coerceIn(0.0, 1.0))
    } else {
      val (l1, oa1, ob1) = cssColorToOklab(c1)
      val (l2, oa2, ob2) = cssColorToOklab(c2)
      val l = (w1 * a1 * l1 + w2 * a2 * l2) / pmAlpha
      val oa = (w1 * a1 * oa1 + w2 * a2 * oa2) / pmAlpha
      val ob = (w1 * a1 * ob1 + w2 * a2 * ob2) / pmAlpha
      oklabToCssColor(l, oa, ob, (pmAlpha * alphaMult).coerceIn(0.0, 1.0))
    }
    return outColor.toCssString()
  }

  private fun buildDynamicRgba(varName: String, alpha: Double): String {
    val fallback = when (varName) {
      "--chip" -> rootVars["--ink-2"]?.toRgbTriplet() ?: "201, 179, 193"
      "--tone" -> rootVars["--success"]?.toRgbTriplet() ?: "64, 219, 128"
      "--glow" -> "100, 100, 160"
      else -> "200, 200, 200"
    }
    return "rgba(var($varName-rgb, $fallback), ${formatAlpha(alpha)})"
  }

  private fun extractSimpleVarName(expr: String): String? {
    val m = Regex("""^var\(\s*(--[a-zA-Z0-9_-]+)\s*\)$""").matchEntire(expr.trim()) ?: return null
    return m.groupValues[1]
  }

  private fun splitTopLevelCommas(s: String): List<String> {
    val out = ArrayList<String>(4)
    var depth = 0
    var start = 0
    var inSingle = false
    var inDouble = false
    for (i in s.indices) {
      val ch = s[i]
      if (ch == '\\') continue
      if (!inDouble && ch == '\'') inSingle = !inSingle
      else if (!inSingle && ch == '"') inDouble = !inDouble
      else if (!inSingle && !inDouble) {
        if (ch == '(') depth++
        else if (ch == ')') depth--
        else if (ch == ',' && depth == 0) {
          out.add(s.substring(start, i))
          start = i + 1
        }
      }
    }
    out.add(s.substring(start))
    return out
  }

  private fun parseMixPart(raw: String): MixPart? {
    val tokens = splitWhitespaceTokens(raw.trim())
    if (tokens.isEmpty()) return null
    if (tokens.size == 1) {
      return MixPart(tokens[0], null)
    }
    if (tokens.size == 2) {
      val t0 = tokens[0]
      val t1 = tokens[1]
      if (t1.endsWith("%")) {
        val p = t1.dropLast(1).toDoubleOrNull() ?: return null
        return MixPart(t0, p)
      }
      if (t0.endsWith("%")) {
        val p = t0.dropLast(1).toDoubleOrNull() ?: return null
        return MixPart(t1, p)
      }
    }
    return null
  }

  private fun injectRgbCompanionVars(css: String): String {
    // First pass: collect all custom property names that represent colors anywhere in this stylesheet.
    val localRawDecls = ArrayList<Pair<String, String>>()
    scanRuleDeclarations(css) { name, value ->
      if (name.startsWith("--") && !name.endsWith("-rgb")) {
        localRawDecls.add(name to value)
      }
    }
    for (pass in 0 until 4) {
      var changed = false
      for ((name, value) in localRawDecls) {
        if (colorVarNames.contains(name)) continue
        val refName = extractSimpleVarName(value)
        val isColor = (refName != null && colorVarNames.contains(refName)) ||
          parseCssColor(value, rootVars) != null
        if (isColor) {
          colorVarNames.add(name)
          changed = true
        }
      }
      if (!changed) break
    }

    // Second pass: rewrite declarations inside { ... } blocks to append `;--name-rgb:...`
    val sb = StringBuilder(css.length + 1024)
    var i = 0
    var inSingle = false
    var inDouble = false
    while (i < css.length) {
      val ch = css[i]
      if (ch == '\\' && i + 1 < css.length) {
        sb.append(css, i, i + 2)
        i += 2
        continue
      }
      if (!inDouble && ch == '\'') {
        inSingle = !inSingle
        sb.append(ch)
        i++
      } else if (!inSingle && ch == '"') {
        inDouble = !inDouble
        sb.append(ch)
        i++
      } else if (!inSingle && !inDouble && ch == '{') {
        val close = findMatchingBrace(css, i)
        if (close < 0) {
          sb.append(css, i, css.length)
          break
        }
        val inner = css.substring(i + 1, close)
        if (hasUnquotedBrace(inner)) {
          // Nested rule/at-rule (e.g. @media { .foo { ... } })
          sb.append('{')
          sb.append(injectRgbCompanionVars(inner))
          sb.append('}')
        } else {
          sb.append('{')
          sb.append(rewriteBlockDeclarations(inner))
          sb.append('}')
        }
        i = close + 1
      } else {
        sb.append(ch)
        i++
      }
    }
    return sb.toString()
  }

  private fun hasUnquotedBrace(s: String): Boolean {
    var i = 0
    var inSingle = false
    var inDouble = false
    while (i < s.length) {
      val ch = s[i]
      if (ch == '\\' && i + 1 < s.length) {
        i += 2
        continue
      }
      if (!inDouble && ch == '\'') inSingle = !inSingle
      else if (!inSingle && ch == '"') inDouble = !inDouble
      else if (!inSingle && !inDouble) {
        if (ch == '/' && i + 1 < s.length && s[i + 1] == '*') {
          val end = s.indexOf("*/", i + 2)
          i = if (end < 0) s.length else end + 2
          continue
        }
        if (ch == '{') return true
      }
      i++
    }
    return false
  }

  private fun scanRuleDeclarations(css: String, onDecl: (String, String) -> Unit) {
    var i = 0
    var inSingle = false
    var inDouble = false
    while (i < css.length) {
      val ch = css[i]
      if (ch == '\\' && i + 1 < css.length) {
        i += 2
        continue
      }
      if (!inDouble && ch == '\'') inSingle = !inSingle
      else if (!inSingle && ch == '"') inDouble = !inDouble
      else if (!inSingle && !inDouble && ch == '{') {
        val close = findMatchingBrace(css, i)
        if (close < 0) break
        val inner = css.substring(i + 1, close)
        if (hasUnquotedBrace(inner)) {
          scanRuleDeclarations(inner, onDecl)
        } else {
          for (d in splitTopLevelDeclarations(inner)) {
            onDecl(d.name, d.value)
          }
        }
        i = close + 1
        continue
      }
      i++
    }
  }

  private fun rewriteBlockDeclarations(blockBody: String): String {
    if (!blockBody.contains("--")) return blockBody
    val decls = splitTopLevelDeclarations(blockBody)
    if (decls.isEmpty()) return blockBody

    val existingNames = HashSet<String>(decls.size)
    for (d in decls) existingNames.add(d.name)

    val sb = StringBuilder(blockBody.length + 64)
    var pos = 0
    for (d in decls) {
      sb.append(blockBody, pos, d.valueEnd)
      if (d.name.startsWith("--") && !d.name.endsWith("-rgb") && colorVarNames.contains(d.name)) {
        val companion = "${d.name}-rgb"
        if (!existingNames.contains(companion)) {
          val refName = extractSimpleVarName(d.value)
          val concrete = parseCssColor(d.value, rootVars)
          val rhs = when {
            refName != null && colorVarNames.contains(refName) -> {
              val fb = concrete?.toRgbTriplet()
              if (fb != null) "var($refName-rgb, $fb)" else "var($refName-rgb)"
            }
            concrete != null -> concrete.toRgbTriplet()
            else -> null
          }
          if (rhs != null) {
            sb.append(';').append(companion).append(':').append(rhs)
          }
        }
      }
      pos = d.valueEnd
    }
    sb.append(blockBody, pos, blockBody.length)
    return sb.toString()
  }

  fun parseCssColor(expr: String, vars: Map<String, CssColor>, depth: Int = 0): CssColor? {
    if (depth > 8) return null
    val t = expr.trim()
    if (t.isEmpty()) return null
    val lower = t.lowercase(Locale.US)
    if (lower == "transparent") return CssColor(0, 0, 0, 0.0)
    if (lower == "black") return CssColor(0, 0, 0, 1.0)
    if (lower == "white") return CssColor(255, 255, 255, 1.0)
    if (t.startsWith("#")) return parseHexColor(t)

    if (lower.startsWith("var(") && t.endsWith(")")) {
      val inner = t.substring(4, t.length - 1)
      val args = splitTopLevelCommas(inner)
      val varName = args[0].trim()
      vars[varName]?.let { return it }
      if (args.size >= 2) {
        return parseCssColor(args[1], vars, depth + 1)
      }
      return null
    }

    if ((lower.startsWith("rgb(") || lower.startsWith("rgba(")) && t.endsWith(")")) {
      val open = t.indexOf('(')
      return parseRgbFunction(t.substring(open + 1, t.length - 1))
    }

    return null
  }

  private fun parseHexColor(hex: String): CssColor? {
    val h = hex.removePrefix("#")
    return when (h.length) {
      3 -> {
        val r = hexNibble(h[0]) ?: return null
        val g = hexNibble(h[1]) ?: return null
        val b = hexNibble(h[2]) ?: return null
        CssColor(r * 17, g * 17, b * 17, 1.0)
      }
      4 -> {
        val r = hexNibble(h[0]) ?: return null
        val g = hexNibble(h[1]) ?: return null
        val b = hexNibble(h[2]) ?: return null
        val a = hexNibble(h[3]) ?: return null
        CssColor(r * 17, g * 17, b * 17, (a * 17) / 255.0)
      }
      6 -> {
        val r = h.substring(0, 2).toIntOrNull(16) ?: return null
        val g = h.substring(2, 4).toIntOrNull(16) ?: return null
        val b = h.substring(4, 6).toIntOrNull(16) ?: return null
        CssColor(r, g, b, 1.0)
      }
      8 -> {
        val r = h.substring(0, 2).toIntOrNull(16) ?: return null
        val g = h.substring(2, 4).toIntOrNull(16) ?: return null
        val b = h.substring(4, 6).toIntOrNull(16) ?: return null
        val a = h.substring(6, 8).toIntOrNull(16) ?: return null
        CssColor(r, g, b, a / 255.0)
      }
      else -> null
    }
  }

  private fun hexNibble(ch: Char): Int? = when (ch) {
    in '0'..'9' -> ch - '0'
    in 'a'..'f' -> ch - 'a' + 10
    in 'A'..'F' -> ch - 'A' + 10
    else -> null
  }

  private fun parseRgbFunction(inner: String): CssColor? {
    if (inner.contains(",")) {
      val parts = inner.split(',').map { it.trim() }
      if (parts.size != 3 && parts.size != 4) return null
      val r = parseRgbChannel(parts[0]) ?: return null
      val g = parseRgbChannel(parts[1]) ?: return null
      val b = parseRgbChannel(parts[2]) ?: return null
      val a = if (parts.size == 4) {
        (parsePercentOrNumber(parts[3], 1.0) ?: return null).coerceIn(0.0, 1.0)
      } else 1.0
      return CssColor(r, g, b, a)
    }
    val slashParts = splitTopLevelSlash(inner) ?: return null
    val tokens = splitWhitespaceTokens(slashParts.first)
    if (tokens.size != 3) return null
    val r = parseRgbChannel(tokens[0]) ?: return null
    val g = parseRgbChannel(tokens[1]) ?: return null
    val b = parseRgbChannel(tokens[2]) ?: return null
    val a = if (slashParts.second != null) {
      (parsePercentOrNumber(slashParts.second!!, 1.0) ?: return null).coerceIn(0.0, 1.0)
    } else 1.0
    return CssColor(r, g, b, a)
  }

  private fun parseRgbChannel(token: String): Int? {
    val t = token.trim()
    return if (t.endsWith("%")) {
      t.dropLast(1).toDoubleOrNull()?.let { ((it / 100.0) * 255.0).roundToInt().coerceIn(0, 255) }
    } else {
      t.toDoubleOrNull()?.roundToInt()?.coerceIn(0, 255)
    }
  }

  private fun formatAlpha(a: Double): String {
    val rounded = (a * 1000.0).roundToInt() / 1000.0
    val s = String.format(Locale.US, "%.3f", rounded)
    return s.trimEnd('0').trimEnd('.')
  }
}
