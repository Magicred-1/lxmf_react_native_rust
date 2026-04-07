plugins {
  id 'com.android.library'
  id 'kotlin-android'
  id 'org.jetbrains.kotlin.android'
}

android {
  compileSdk 34
  namespace 'expo.modules.lxmf'

  defaultConfig {
    minSdk 24
    targetSdk 34
    compileSdk 34
  }

  compileOptions {
    sourceCompatibility JavaVersion.VERSION_11
    targetCompatibility JavaVersion.VERSION_11
  }

  kotlinOptions {
    jvmTarget = '11'
  }

  sourceSets {
    main {
      java.srcDirs += 'src/main/kotlin'
    }
  }
}

dependencies {
  implementation 'org.jetbrains.kotlin:kotlin-stdlib-jdk8:1.9.0'
  implementation 'expo:expo-modules-core:1.11.0'
}

// Link the Rust library
task copyRustLibraries {
  doLast {
    def rustLibDir = "../../../../rust-core/target/release"
    copy {
      from rustLibDir
      include "liblxmf_rn.so"
      into "src/main/jniLibs/arm64-v8a"
    }
  }
}

preBuild.dependsOn copyRustLibraries
