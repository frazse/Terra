# Vendored: Hammerhead karoo-ext 1.1.9

`lib/src` is an unmodified copy of the `karoo-ext` library (the official Karoo
extension framework), provided by SRAM LLC / Hammerhead Nav:

- Upstream: https://github.com/hammerheadnav/karoo-ext (tag 1.1.9)
- Copyright (c) 2024-2026 SRAM LLC.
- License: Apache License 2.0 (each file carries the full license header).

The normal way to consume this framework is the Maven artifact
`io.hammerhead:karoo-ext:1.1.9`, which is only published to GitHub Packages
(`maven.pkg.github.com/hammerheadnav/karoo-ext`) and requires GitHub
authenticated credentials — so we vendor the source here instead.

Only one build-side piece from the upstream `lib/build.gradle.kts` is
re-created in this module's build file (no source patches):

- `buildConfigField("String", "LIB_VERSION", "\"1.1.9\"")` — upstream injects
  this into `io.hammerhead.karooext.BuildConfig`, which `Constants.kt` reads.

Dependencies used by the vendored code: `androidx.core:core-ktx`,
`org.jetbrains.kotlinx:kotlinx-serialization-json`, `com.jakewharton.timber:timber`.
