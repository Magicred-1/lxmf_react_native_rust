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

// Copy Rust library
val copyRustLibraries by tasks.registering(Copy::class) {
    val rustLibDir = file("../../rust-core/target/aarch64-linux-android/release")
    val rustLib = rustLibDir.resolve("liblxmf_rn.so")

    doFirst {
        if (!rustLib.exists()) {
            throw GradleException(
                "Missing Android Rust library at ${rustLib.absolutePath}. Build it first for target aarch64-linux-android."
            )
        }
    }

    from(rustLibDir)
    include("liblxmf_rn.so")
    into("src/main/jniLibs/arm64-v8a")
}

tasks.named("preBuild") {
    dependsOn(copyRustLibraries)
}