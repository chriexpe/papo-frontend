import java.security.MessageDigest

plugins {
    alias(libs.plugins.android.application)
    id("org.jetbrains.kotlin.plugin.compose")
}

val cargoVersion = rootProject.file("../Cargo.toml").readLines()
    .first { it.startsWith("version = ") }.substringAfter('=') .trim().trim('"')
val parts = cargoVersion.substringBefore('-').split('.').map(String::toInt)
val papoAbis = providers.gradleProperty("papoAbis").orElse("arm64-v8a,x86_64").get()
    .split(',').map(String::trim)
require(papoAbis.all { it in setOf("arm64-v8a", "x86_64") }) { "Unsupported papoAbis" }

android {
    namespace = "io.github.chriexpe.papo.compose"
    compileSdk = 37
    ndkVersion = "30.0.16248370"
    defaultConfig {
        applicationId = "io.github.chriexpe.papo.compose"
        minSdk = 31
        targetSdk = 37
        versionCode = parts[0] * 1_000_000 + parts[1] * 1_000 + parts[2]
        versionName = cargoVersion
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        testInstrumentationRunnerArguments["class"] = "io.github.chriexpe.papo.compose.AllTests"
        ndk { abiFilters += papoAbis }
    }
    buildFeatures { compose = true; buildConfig = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    packaging { jniLibs.excludes += setOf("**/libturso_sdk_kit-*.so", "**/libturso_sync_sdk_kit-*.so") }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended:1.7.8")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.4")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.4")
    // Captures the content layer once and samples it behind the floating
    // controls. This is backdrop blur, not a blur applied to the text/controls.
    implementation("dev.chrisbanes.haze:haze:1.6.10")
    implementation("net.java.dev.jna:jna:5.18.1@aar")
    implementation("io.coil-kt.coil3:coil-compose:3.3.0")
    implementation("io.coil-kt.coil3:coil-gif:3.3.0")
    implementation("androidx.media3:media3-exoplayer:1.11.1")
    implementation("androidx.media3:media3-exoplayer-hls:1.11.1")
    implementation("androidx.media3:media3-ui:1.11.1")
    implementation("androidx.media3:media3-datasource-okhttp:1.11.1")
    debugImplementation("androidx.compose.ui:ui-tooling")
    androidTestImplementation(platform("androidx.compose:compose-bom:2026.09.00"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    // Android 17 removed the hidden InputManager.getInstance API used by
    // older Espresso versions. This test-only release uses getSystemService.
    androidTestImplementation("androidx.test.espresso:espresso-core:3.8.0-alpha01")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// Android Studio and Gradle builds must build the real shared core too.
// -PpapoAbis=x86_64 speeds up emulator-only builds; default APKs support both.
val rustJni = layout.buildDirectory.dir("rustJniLibs")
android.sourceSets["main"].jniLibs.setSrcDirs(listOf(rustJni))
val buildRustNative by tasks.registering(Exec::class) {
    group = "build"
    description = "Build papo-core and its UniFFI adapter without egui or GStreamer"
    workingDir(rootProject.file(".."))
    inputs.files(rootProject.file("../Cargo.toml"), rootProject.file("../Cargo.lock"))
    inputs.files(fileTree(rootProject.file("../crates/papo-core")) { include("Cargo.toml", "src/**") })
    inputs.files(fileTree(rootProject.file("../crates/papo-android")) { include("Cargo.toml", "src/**") })
    inputs.files(fileTree("src/main/assets/demo"))
    inputs.property("abis", papoAbis)
    inputs.property("gifProviderConfiguration", providers.environmentVariable("PAPO_GIPHY_API_KEY").map {
        MessageDigest.getInstance("SHA-256").digest(it.toByteArray()).joinToString("") { byte -> "%02x".format(byte) }
    }.orElse("unconfigured"))
    inputs.file(rootProject.file("../crates/papo-core/build.rs"))
    papoAbis.forEach { outputs.file(rustJni.map { directory -> directory.file("$it/libpapo_compose.so") }) }
    commandLine(listOf("cargo", "ndk") + papoAbis.flatMap { listOf("-t", it) } +
        listOf("--platform", "31", "-o", rustJni.get().asFile.absolutePath, "build", "--locked", "-p", "papo-android", "--lib"))
    doFirst {
        environment("ANDROID_NDK_HOME", androidComponents.sdkComponents.sdkDirectory.get().asFile.resolve("ndk/${android.ndkVersion}").absolutePath)
    }
}
val generateUniFFI by tasks.registering(Exec::class) {
    dependsOn(buildRustNative)
    workingDir(rootProject.file(".."))
    inputs.file(rootProject.file("../crates/papo-android/uniffi.toml"))
    inputs.file(rustJni.map { it.file("${papoAbis.first()}/libpapo_compose.so") })
    outputs.dir(layout.buildDirectory.dir("generated/uniffi"))
    commandLine("cargo", "run", "--locked", "-p", "papo-android", "--features", "bindgen", "--bin", "papo-bindgen", "--",
        "generate", "--library", rustJni.get().asFile.resolve("${papoAbis.first()}/libpapo_compose.so").absolutePath,
        "--language", "kotlin", "--config", rootProject.file("../crates/papo-android/uniffi.toml").absolutePath,
        "--out-dir", layout.buildDirectory.dir("generated/uniffi").get().asFile.absolutePath)
}
android.sourceSets["main"].kotlin.srcDir(layout.buildDirectory.dir("generated/uniffi").get().asFile)
tasks.named("preBuild") { dependsOn(generateUniFFI) }
