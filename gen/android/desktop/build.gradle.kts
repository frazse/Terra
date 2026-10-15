plugins {
    kotlin("jvm")
    id("org.jetbrains.compose")
    id("org.jetbrains.kotlin.plugin.compose")
}

repositories {
    google()
    mavenCentral()
    maven("https://maven.pkg.jetbrains.space/public/p/compose/dev")
}

dependencies {
    implementation(compose.desktop.currentOs)
    implementation("me.friwi:jcefmaven:110.0.25")
    implementation("com.github.hypfvieh:bluez-dbus:0.1.4")
    implementation("com.github.hypfvieh:dbus-java:3.3.0")
    implementation("org.json:json:20240303")
}

compose.desktop {
    application {
        mainClass = "dev.frazse.terra.desktop.MainKt"
        nativeDistributions {
            targetFormats(org.jetbrains.compose.desktop.application.dsl.TargetFormat.AppImage)
            packageName = "Terra"
            packageVersion = "0.4.1"
        }
    }
}
