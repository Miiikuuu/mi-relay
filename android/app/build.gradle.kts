plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// Public release metadata only. Signing keys/passwords never enter Gradle or CI.
val releaseVersion = providers.gradleProperty("mirelayReleaseVersion").orNull
val releaseCode = providers.gradleProperty("mirelayReleaseVersionCode").orNull
fun packageVersion(path: String): String? {
    val section = Regex("(?ms)^\\[package]\\s*\\n(.*?)(?=^\\[|\\z)")
        .find(rootProject.file(path).readText())?.groupValues?.get(1) ?: return null
    return Regex("(?m)^version\\s*=\\s*\"([^\"]+)\"\\s*$").find(section)?.groupValues?.get(1)
}
val validateReleaseMetadata = tasks.register("validateReleaseMetadata") {
    doLast {
        check(releaseVersion?.matches(Regex("[0-9]+\\.[0-9]+\\.[0-9]+(-(alpha|beta|rc)\\.[0-9]+)?")) == true) {
            "Use android/scripts/release.py prepare with explicit release metadata."
        }
        check(releaseCode?.toIntOrNull()?.let { it in 2..2100000000 } == true) {
            "A release versionCode must be explicitly selected (2..2100000000)."
        }
        check(releaseVersion == packageVersion("../Cargo.toml") && releaseVersion == packageVersion("native/Cargo.toml")) {
            "Release versionName must match both Cargo package versions."
        }
    }
}
tasks.matching { it.name == "preReleaseBuild" }.configureEach {
    dependsOn(validateReleaseMetadata)
}

android {
    namespace = "io.mirelay.android"
    compileSdk = 36
    ndkVersion = "27.2.12479018"
    defaultConfig {
        applicationId = "io.mirelay.android"
        minSdk = 26
        targetSdk = 36
        versionCode = releaseCode?.toIntOrNull() ?: 1
        versionName = releaseVersion ?: "0.1.0-dev"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures { compose = true; buildConfig = true }
    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = null // Prepare unsigned; a separate local tool signs a copy.
        }
    }
    testOptions {
        unitTests.isIncludeAndroidResources = true
        unitTests.all {
            val testHome = rootProject.file(".local/test-user")
            it.doFirst { check(testHome.mkdirs() || testHome.isDirectory) }
            it.systemProperty("user.home", testHome.absolutePath)
            it.systemProperty("maven.repo.local", rootProject.file(".local/robolectric").absolutePath)
            it.systemProperty("robolectric.dependency.repo.url", "https://repo.maven.apache.org/maven2")
            System.getProperty("mirelay.qr.fixture")?.let { path -> it.systemProperty("mirelay.qr.fixture", path) }
            System.getProperty("https.proxyHost")?.let { host -> it.systemProperty("robolectric.dependency.proxy.host", host) }
            System.getProperty("https.proxyPort")?.let { port -> it.systemProperty("robolectric.dependency.proxy.port", port) }
        }
    }
    lint { abortOnError = true }
}

kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }

dependencies {
    implementation(files(rootProject.file(".local/rustls-platform-verifier.aar")))
    implementation(platform("androidx.compose:compose-bom:2025.09.01"))
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")
    implementation("androidx.activity:activity-compose:1.11.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.4")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.4")
    implementation("androidx.work:work-runtime-ktx:2.10.5")
    implementation("androidx.core:core-ktx:1.17.0")
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.16")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("androidx.test:rules:1.7.0")
    androidTestImplementation(platform("androidx.compose:compose-bom:2025.09.01"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.3.0")
}
