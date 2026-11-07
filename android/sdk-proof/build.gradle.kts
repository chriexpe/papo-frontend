plugins {
    alias(libs.plugins.android.application)
}

val nativeOutput = layout.buildDirectory.dir("sdk-proof-native")
val generatedBindings = layout.buildDirectory.dir("generated/uniffi")

android {
    namespace = "io.github.chriexpe.papo.sdkproof"
    compileSdk = 37
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "io.github.chriexpe.papo.sdkproof"
        minSdk = 31
        targetSdk = 37
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += "x86_64" }
    }

    sourceSets["main"].java.srcDir(generatedBindings)
    sourceSets["main"].jniLibs.srcDir(nativeOutput)

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation("net.java.dev.jna:jna:5.18.1@aar")
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.runtime:runtime")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}

val buildSdkProofNative by tasks.registering(Exec::class) {
    group = "verification"
    description = "Build only the new SDK UniFFI integration fixture for the emulator"
    workingDir(rootProject.file(".."))
    inputs.files(
        fileTree(rootProject.file("../crates/sdk/ffi")) { include("Cargo.toml", "src/**") },
        fileTree(rootProject.file("../crates/sdk/ui")) { include("Cargo.toml", "src/**") },
        rootProject.file("../Cargo.lock"),
    )
    outputs.file(nativeOutput.map { it.file("x86_64/libpapo_ffi.so") })
    commandLine(
        "bash",
        "scripts/android-sdk-proof-native.sh",
        nativeOutput.get().asFile.absolutePath,
    )
}

val generateSdkProofBindings by tasks.registering(Exec::class) {
    group = "verification"
    description = "Generate Kotlin from the exact SDK proof library packaged in this APK"
    dependsOn(buildSdkProofNative)
    workingDir(rootProject.file(".."))
    inputs.file(nativeOutput.map { it.file("x86_64/libpapo_ffi.so") })
    inputs.file(rootProject.file("../crates/sdk/ffi/uniffi.toml"))
    outputs.dir(generatedBindings)
    commandLine(
        "cargo",
        "run",
        "--locked",
        "-p",
        "papo-ffi",
        "--features",
        "bindgen,integration-proof",
        "--bin",
        "papo-sdk-bindgen",
        "--",
        "generate",
        "--library",
        nativeOutput.get().asFile.resolve("x86_64/libpapo_ffi.so").absolutePath,
        "--language",
        "kotlin",
        "--out-dir",
        generatedBindings.get().asFile.absolutePath,
        "--config",
        rootProject.file("../crates/sdk/ffi/uniffi.toml").absolutePath,
    )
}

tasks.matching { it.name == "preDebugBuild" }.configureEach {
    dependsOn(generateSdkProofBindings)
}
