package dev.frazse.terra

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CssCompatTest {

  @Test
  fun convertsOklchAndAlpha() {
    val css = """
      :root{--bt-7:#ffcd7c;--sunk:oklch(25.5% .03 318);--ground:oklch(29% .032 318);--ink:oklch(93% .014 70);--ink-2:oklch(80% .022 340);--success:oklch(80% .16 150);--danger:oklch(74% .15 25)}
      .sunset{background:radial-gradient(30% 110% at var(--sun-x,62%) 105%, oklch(86% .13 75/.95) 0 14%, transparent 70%)}
      .sunset:before{background:oklch(30% .06 320/.85)}
    """.trimIndent()

    val out = CssCompat.transformCss(css)
    assertFalse("Should not contain oklch()", out.contains("oklch(", ignoreCase = true))
    assertTrue("Should contain rgb() for --sunk", out.contains("--sunk:rgb("))
    assertTrue("Should contain --sunk-rgb companion", out.contains("--sunk-rgb:"))
    assertTrue("Should contain rgba() for .95 alpha", out.contains("rgba(") && out.contains("0.95)"))
    assertTrue("Should contain rgba() for .85 alpha", out.contains("0.85)"))
  }

  @Test
  fun convertsStaticAndDynamicColorMix() {
    val css = """
      :root{--bt-e:#572667;--bt-7:#ffcd7c;--focus:var(--bt-7);--sunk:oklch(25.5% .03 318);--ground:oklch(29% .032 318);--surface:oklch(34% .036 318);--ink:oklch(93% .014 70);--ink-2:oklch(80% .022 340);--success:oklch(80% .16 150);--danger:oklch(74% .15 25);--selected:color-mix(in oklab, var(--focus) 12%, var(--sunk))}
      .card{background:linear-gradient(180deg, var(--ground) 20%, color-mix(in oklab, var(--bt-e) 22%, var(--ground)) 60%)}
      .ridge{fill:color-mix(in oklab, var(--ground), var(--sunk))}
      .hover:hover{background:color-mix(in oklab, var(--ink) 6%, transparent)}
      .sky-flat{--horizon:oklch(68% .09 230);--glow:oklch(42% .08 250)}
      .sky-banner{background:radial-gradient(85% 120% at 50% 112%, var(--horizon), color-mix(in oklab, var(--glow) 80%, transparent) 42%, transparent 78%), var(--ground)}
      .btn-success,.btn-danger{background:color-mix(in oklab, var(--tone) 16%, transparent);color:var(--tone)}
      .btn-success{--tone:var(--success)}
      .btn-danger{--tone:var(--danger)}
      .pill{--chip:var(--ink-2);background:color-mix(in oklab, var(--chip) 14%, transparent);color:var(--chip)}
      .type-race{--chip:var(--type-race)}
      .behind{background:color-mix(in srgb, var(--bt-8) 25%, transparent)}
    """.trimIndent()

    val out = CssCompat.transformCss(css)
    assertFalse("Should not contain oklch()", out.contains("oklch(", ignoreCase = true))
    assertFalse("Should not contain color-mix()", out.contains("color-mix(", ignoreCase = true))
    assertTrue("Should resolve --selected", out.contains("--selected:rgb("))
    assertTrue("Should emit --glow-rgb on .sky-flat", out.contains("--glow-rgb:"))
    assertTrue("Should emit --tone-rgb:var(--success-rgb", out.contains("--tone-rgb:var(--success-rgb"))
    assertTrue("Should emit --chip-rgb:var(--ink-2-rgb", out.contains("--chip-rgb:var(--ink-2-rgb"))
    assertTrue("Should use rgba(var(--tone-rgb...) on .btn-success", out.contains("rgba(var(--tone-rgb"))
    assertTrue("Should use rgba(var(--glow-rgb...) on .sky-banner", out.contains("rgba(var(--glow-rgb"))
    assertTrue("Should resolve srgb mix in .behind", out.contains(".behind{background:rgba(255, 165, 116, 0.25)}"))
  }

  @Test
  fun leavesNonColorVariablesAndDataUrlsUntouched() {
    val css = """
      :root{--font:"Overpass", sans-serif;--radius-sm:.25rem;--chevron:url("data:image/svg+xml,%3Csvg%3E%3C/svg%3E")}
    """.trimIndent()
    val out = CssCompat.transformCss(css)
    assertFalse("Should not add --font-rgb", out.contains("--font-rgb"))
    assertFalse("Should not add --radius-sm-rgb", out.contains("--radius-sm-rgb"))
    assertFalse("Should not add --chevron-rgb", out.contains("--chevron-rgb"))
  }
}
