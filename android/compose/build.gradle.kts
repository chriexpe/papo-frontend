import java.security.MessageDigest

plugins {
    alias(libs.plugins.android.application)
    id("org.jetbrains.kotlin.plugin.compose")
}

val cargoVersion =
    rootProject
        .file("../Cargo.toml")
        .readLines()
        .first { it.startsWith("version = ") }
        .substringAfter('=')
        .trim()
        .trim('"')
val parts = cargoVersion.substringBefore('-').split('.').map(String::toInt)
val papoAbis =
    providers
        .gradleProperty("papoAbis")
        .orElse("arm64-v8a,x86_64")
        .get()
        .split(',')
        .map(String::trim)

require(papoAbis.all { it in setOf("arm64-v8a", "x86_64") }) { "Unsupported papoAbis" }

// Official releases retain Papo's existing Android identity and Firebase app.
// Debug builds stay beside it, preserving Compose development installations.
fun firebaseValue(name: String): String? =
    (System.getenv(name) ?: providers.gradleProperty(name).orNull)?.takeIf { it.isNotBlank() }

fun firebaseConfig(debug: Boolean) =
    mapOf(
        "google_app_id" to
            firebaseValue(if (debug) "PAPO_COMPOSE_FIREBASE_APP_ID" else "PAPO_FIREBASE_APP_ID"),
        "google_api_key" to firebaseValue("PAPO_FIREBASE_API_KEY"),
        "project_id" to firebaseValue("PAPO_FIREBASE_PROJECT_ID"),
        "gcm_defaultSenderId" to firebaseValue("PAPO_FIREBASE_SENDER_ID"),
    )

val releaseKeystore = firebaseValue("PAPO_ANDROID_KEYSTORE")

android {
    sourceSets["main"].java.srcDir(rootProject.file("shared/src/main/java"))
    sourceSets["main"].java.srcDir(rootProject.file("app/src/main/java/org"))
    sourceSets["main"].assets.srcDir(layout.buildDirectory.dir("gstreamer-assets").get().asFile)
    namespace = "io.github.chriexpe.papo.compose"
    compileSdk = 37
    ndkVersion = "30.0.16248370"
    defaultConfig {
        applicationId = "io.github.chriexpe.papo"
        minSdk = 31
        targetSdk = 37
        versionCode = parts[0] * 1_000_000 + parts[1] * 1_000 + parts[2]
        versionName = cargoVersion
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        testInstrumentationRunnerArguments["class"] = "io.github.chriexpe.papo.compose.AllTests"
        ndk { abiFilters += papoAbis }
    }
    signingConfigs {
        if (releaseKeystore != null)
            create("release") {
                storeFile = file(releaseKeystore)
                storePassword =
                    firebaseValue("PAPO_ANDROID_STORE_PASSWORD")
                        ?: error("Missing Android store password")
                keyAlias =
                    firebaseValue("PAPO_ANDROID_KEY_ALIAS") ?: error("Missing Android key alias")
                keyPassword =
                    firebaseValue("PAPO_ANDROID_KEY_PASSWORD")
                        ?: error("Missing Android key password")
            }
    }
    buildTypes {
        getByName("debug") { applicationIdSuffix = ".compose" }
        getByName("release") {
            isMinifyEnabled = false
            if (releaseKeystore != null) signingConfig = signingConfigs.getByName("release")
        }
        create("benchmark") {
            initWith(getByName("release"))
            isDebuggable = false
            applicationIdSuffix = ".compose"
            versionNameSuffix = "-benchmark"
            signingConfig = signingConfigs.getByName("debug")
            matchingFallbacks += listOf("release")
        }
        all {
            val debug = name != "release"
            val config = firebaseConfig(debug)
            val configured = config.values.all { it != null }
            val required =
                firebaseValue(
                    if (debug) "PAPO_COMPOSE_REQUIRE_FIREBASE" else "PAPO_REQUIRE_FIREBASE"
                ) == "1"
            if (!configured && required) error("Firebase configuration is required for $name")
            buildConfigField("boolean", "FIREBASE_CONFIGURED", configured.toString())
            buildConfigField(
                "String",
                "FIREBASE_CONFIGURATION",
                "\"" +
                    MessageDigest.getInstance("SHA-256")
                        .digest(config.values.joinToString().toByteArray())
                        .joinToString("") { "%02x".format(it) } +
                    "\"",
            )
            if (configured) config.forEach { (key, value) -> resValue("string", key, value!!) }
            resValue("string", "app_name", if (debug) "Papo Compose" else "Papo")
        }
    }
    // Both supported languages must remain available when an installed app changes locale offline.
    bundle { language { enableSplit = false } }
    buildFeatures {
        compose = true
        buildConfig = true
        resValues = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    packaging {
        jniLibs.excludes += setOf("**/libturso_sdk_kit-*.so", "**/libturso_sync_sdk_kit-*.so")
    }
}

dependencies {
    implementation(libs.work.runtime)
    implementation(platform(libs.firebase.bom))
    implementation(libs.firebase.messaging)
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
// Keep native debug/release artifacts and bindings separate: the official
// release must not accidentally package a development-profile Rust library.
listOf("debug", "release", "benchmark").forEach { variant ->
    val release = variant != "debug"
    val capitalized = variant.replaceFirstChar { it.uppercaseChar() }
    val suffix = if (variant == "debug") "" else capitalized
    val nativeDirectory = layout.buildDirectory.dir("rust${suffix}JniLibs")
    val generatedDirectory = layout.buildDirectory.dir("generated/uniffi/$variant")
    android.sourceSets[variant].jniLibs.setSrcDirs(listOf(nativeDirectory.get().asFile))
    android.sourceSets[variant].kotlin.srcDir(generatedDirectory.get().asFile)
    val native =
        tasks.register<Exec>("buildRust${suffix}Native") {
            group = "build"
            description =
                "Build $variant papo-core, shared GStreamer call engine and UniFFI without egui"
            workingDir(rootProject.file(".."))
            inputs.files(rootProject.file("../Cargo.toml"), rootProject.file("../Cargo.lock"))
            inputs.files(
                fileTree(rootProject.file("../crates/papo-core")) {
                    include("Cargo.toml", "src/**", "build.rs")
                }
            )
            inputs.files(
                fileTree(rootProject.file("../crates/papo-android")) {
                    include("Cargo.toml", "src/**")
                }
            )
            inputs.files(
                fileTree(rootProject.file("../crates/papo-call")) {
                    include("Cargo.toml", "src/**")
                }
            )
            inputs.files(
                rootProject.file("../scripts/android-compose-native.sh"),
                rootProject.file("call-gstreamer/CMakeLists.txt"),
            )
            inputs.files(fileTree("src/main/assets/demo"))
            inputs.property("abis", papoAbis)
            inputs.property("profile", variant)
            // UniFFI reads the full symbol table before AGP strips the packaged
            // library. Cargo's workspace strip=true removes that metadata table.
            if (release) environment("CARGO_PROFILE_RELEASE_STRIP", "debuginfo")
            inputs.property(
                "gifProviderConfiguration",
                providers
                    .environmentVariable("PAPO_GIPHY_API_KEY")
                    .map {
                        MessageDigest.getInstance("SHA-256").digest(it.toByteArray()).joinToString(
                            ""
                        ) { byte ->
                            "%02x".format(byte)
                        }
                    }
                    .orElse("unconfigured"),
            )
            papoAbis.forEach { abi ->
                listOf("libpapo_compose.so", "libgstreamer_android.so", "libc++_shared.so")
                    .forEach { library ->
                        outputs.file(nativeDirectory.map { it.file("$abi/$library") })
                    }
            }
            commandLine(
                "bash",
                "scripts/android-compose-native.sh",
                if (release) "release" else "debug",
                nativeDirectory.get().asFile.absolutePath,
                papoAbis.joinToString(","),
            )
            doFirst {
                environment(
                    "ANDROID_NDK_HOME",
                    androidComponents.sdkComponents.sdkDirectory
                        .get()
                        .asFile
                        .resolve("ndk/${android.ndkVersion}")
                        .absolutePath,
                )
            }
        }
    val bindings =
        tasks.register<Exec>("generate${suffix}UniFFI") {
            dependsOn(native)
            workingDir(rootProject.file(".."))
            inputs.file(rootProject.file("../crates/papo-android/uniffi.toml"))
            inputs.file(nativeDirectory.map { it.file("${papoAbis.first()}/libpapo_compose.so") })
            outputs.dir(generatedDirectory)
            commandLine(
                "cargo",
                "run",
                "--locked",
                "-p",
                "papo-android",
                "--features",
                "bindgen",
                "--bin",
                "papo-bindgen",
                "--",
                "generate",
                "--library",
                nativeDirectory
                    .get()
                    .asFile
                    .resolve("${papoAbis.first()}/libpapo_compose.so")
                    .absolutePath,
                "--language",
                "kotlin",
                "--out-dir",
                generatedDirectory.get().asFile.absolutePath,
            )
        }
    tasks.matching { it.name == "pre${capitalized}Build" }.configureEach { dependsOn(bindings) }
}

// Format only checked-in Compose sources. UniFFI output belongs to its generator.
val composeFormatter by configurations.creating {
    isCanBeResolved = true
    isCanBeConsumed = false
}

dependencies {
    composeFormatter("com.facebook:ktfmt:0.64:with-dependencies") { isTransitive = false }
}

val composeKotlinSources = fileTree("src") { include("**/*.kt") } + files("build.gradle.kts")

fun registerComposeFormatter(name: String, verify: Boolean) =
    tasks.register<JavaExec>(name) {
        group = "verification"
        description =
            if (verify) "Check Compose Kotlin formatting" else "Format Compose Kotlin sources"
        classpath = composeFormatter
        mainClass.set("com.facebook.ktfmt.cli.Main")
        inputs.files(composeKotlinSources)
        doFirst {
            args =
                listOf("--kotlinlang-style") +
                    (if (verify) listOf("--dry-run", "--set-exit-if-changed") else emptyList()) +
                    composeKotlinSources.files.sortedBy { it.path }.map { it.absolutePath }
        }
    }

registerComposeFormatter("formatCompose", false)

val checkComposeFormatting = registerComposeFormatter("checkComposeFormatting", true)

tasks.named("check") { dependsOn(checkComposeFormatting) }
