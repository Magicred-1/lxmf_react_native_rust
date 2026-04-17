plugins {
    id("com.android.library")
    id("expo-module-gradle-plugin")
}

android {
    namespace = "expo.modules.lxmf"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
        versionCode = 1
        versionName = "1.0.0"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }

    kotlinOptions {
        jvmTarget = "11"
    }

    sourceSets {
        getByName("main").java.srcDirs("src/main/kotlin")
    }
}

dependencies {
    compileOnly(project(":expo-modules-core"))
}

