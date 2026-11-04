plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

android {
    namespace = "dev.bridge.phone"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.bridge.phone"
        minSdk = 29 // Android 10: files are saved through MediaStore, which needs no storage permission
        targetSdk = 35
        // The release workflow passes the version of the tag it builds: -PappVersion=0.3.1 -PappVersionCode=301
        versionCode = (project.findProperty("appVersionCode") as String?)?.toInt() ?: 300
        versionName = (project.findProperty("appVersion") as String?) ?: "0.3.0"
    }

    // Release signing. The key is never in the repository: the release workflow writes it from GitHub secrets
    // and sets these variables (see docs/RELEASING.md). Without them a release build stays unsigned.
    val keystorePath = System.getenv("BRIDGE_KEYSTORE")
    signingConfigs {
        if (keystorePath != null) {
            create("release") {
                storeFile = file(keystorePath)
                storePassword = System.getenv("BRIDGE_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("BRIDGE_KEY_ALIAS")
                keyPassword = System.getenv("BRIDGE_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            if (keystorePath != null) signingConfig = signingConfigs.getByName("release")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
    }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2024.12.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.annotation:annotation-experimental:1.4.1")

    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")

    // QR scanner
    implementation("androidx.camera:camera-core:1.4.1")
    implementation("androidx.camera:camera-camera2:1.4.1")
    implementation("androidx.camera:camera-lifecycle:1.4.1")
    implementation("androidx.camera:camera-view:1.4.1")
    implementation("com.google.mlkit:barcode-scanning:17.3.0")
}
