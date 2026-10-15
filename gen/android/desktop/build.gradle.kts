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
