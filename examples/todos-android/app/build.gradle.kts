import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "lab.reactor.todos"
    compileSdk = 35

    defaultConfig {
        applicationId = "lab.reactor.todos"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"
        val props = Properties()
        val local = rootProject.file("local.properties")
        if (local.exists()) props.load(local.inputStream())
        val url = props.getProperty("REACTOR_URL", "http://10.0.2.2:18000")
        val key = props.getProperty("REACTOR_ANON_KEY", "")
        buildConfigField("String", "REACTOR_URL", "\"$url\"")
        buildConfigField("String", "REACTOR_ANON_KEY", "\"$key\"")
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation(project(":reactor-client"))
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.compose.ui:ui:1.7.6")
    implementation("androidx.compose.material3:material3:1.3.1")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
}
