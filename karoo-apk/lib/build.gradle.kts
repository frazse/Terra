import org.jetbrains.kotlin.gradle.dsl.JvmTarget

// Vendored copy of hammerheadnav/karoo-ext 1.1.9 (Apache-2.0, SRAM LLC).
// See NOTICE.md. Only the buildConfigField LIB_VERSION (normally injected by the
// upstream library build) is re-created here so Constants.kt compiles unmodified.
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.serialization")
}

android {
    compileSdk = 37
    namespace = "io.hammerhead.karooext"
    defaultConfig {
        minSdk = 24
        buildConfigField("String", "LIB_VERSION", "\"1.1.9\"")
    }
    buildTypes {
        getByName("debug") {
            isMinifyEnabled = false
        }
        getByName("release") {
            isMinifyEnabled = false
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
        aidl = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    // `api`: Karoo System deserializes our emitted events with this runtime.
    api("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
    implementation("com.jakewharton.timber:timber:5.0.1")
}
