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

// Official releases retain Papo's existing Android identity and Firebase app.
// Debug builds stay beside it, preserving Compose development installations.
fun firebaseValue(name:String):String? = (System.getenv(name) ?: providers.gradleProperty(name).orNull)?.takeIf {it.isNotBlank()}
fun firebaseConfig(debug:Boolean)=mapOf(
    "google_app_id" to firebaseValue(if(debug) "PAPO_COMPOSE_FIREBASE_APP_ID" else "PAPO_FIREBASE_APP_ID"),
    "google_api_key" to firebaseValue("PAPO_FIREBASE_API_KEY"),
    "project_id" to firebaseValue("PAPO_FIREBASE_PROJECT_ID"),
    "gcm_defaultSenderId" to firebaseValue("PAPO_FIREBASE_SENDER_ID"),
)
val releaseKeystore=firebaseValue("PAPO_ANDROID_KEYSTORE")

android {
    sourceSets["main"].java.srcDir(rootProject.file("shared/src/main/java"))
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
        if(releaseKeystore!=null) create("release") {
            storeFile=file(releaseKeystore)
            storePassword=firebaseValue("PAPO_ANDROID_STORE_PASSWORD") ?: error("Missing Android store password")
            keyAlias=firebaseValue("PAPO_ANDROID_KEY_ALIAS") ?: error("Missing Android key alias")
            keyPassword=firebaseValue("PAPO_ANDROID_KEY_PASSWORD") ?: error("Missing Android key password")
        }
    }
    buildTypes {
        getByName("debug") {applicationIdSuffix=".compose"}
        getByName("release") {
            isMinifyEnabled=false
            if(releaseKeystore!=null) signingConfig=signingConfigs.getByName("release")
        }
        all {
            val debug=name=="debug"
            val config=firebaseConfig(debug)
            val configured=config.values.all {it!=null}
            val required=firebaseValue(if(debug) "PAPO_COMPOSE_REQUIRE_FIREBASE" else "PAPO_REQUIRE_FIREBASE")=="1"
            if(!configured && required) error("Firebase configuration is required for $name")
            buildConfigField("boolean","FIREBASE_CONFIGURED",configured.toString())
            buildConfigField("String","FIREBASE_CONFIGURATION","\""+MessageDigest.getInstance("SHA-256").digest(config.values.joinToString().toByteArray()).joinToString("") {"%02x".format(it)}+"\"")
            if(configured) config.forEach {(key,value)->resValue("string",key,value!!)}
            resValue("string","app_name",if(debug) "Papo Compose" else "Papo")
        }
    }
    buildFeatures { compose = true; buildConfig = true; resValues = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    packaging { jniLibs.excludes += setOf("**/libturso_sdk_kit-*.so", "**/libturso_sync_sdk_kit-*.so") }
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
listOf("debug","release").forEach { variant ->
    val release=variant=="release"
    val suffix=if(release) "Release" else ""
    val nativeDirectory=layout.buildDirectory.dir(if(release) "rustReleaseJniLibs" else "rustJniLibs")
    val generatedDirectory=layout.buildDirectory.dir("generated/uniffi/$variant")
    android.sourceSets[variant].jniLibs.setSrcDirs(listOf(nativeDirectory.get().asFile))
    android.sourceSets[variant].kotlin.srcDir(generatedDirectory.get().asFile)
    val native=tasks.register<Exec>("buildRust${suffix}Native") {
        group="build"
        description="Build $variant papo-core and UniFFI without egui or GStreamer"
        workingDir(rootProject.file(".."))
        inputs.files(rootProject.file("../Cargo.toml"),rootProject.file("../Cargo.lock"))
        inputs.files(fileTree(rootProject.file("../crates/papo-core")) {include("Cargo.toml","src/**","build.rs")})
        inputs.files(fileTree(rootProject.file("../crates/papo-android")) {include("Cargo.toml","src/**")})
        inputs.files(fileTree("src/main/assets/demo"))
        inputs.property("abis",papoAbis)
        inputs.property("profile",variant)
        // UniFFI reads the full symbol table before AGP strips the packaged
        // library. Cargo's workspace strip=true removes that metadata table.
        if(release) environment("CARGO_PROFILE_RELEASE_STRIP","debuginfo")
        inputs.property("gifProviderConfiguration",providers.environmentVariable("PAPO_GIPHY_API_KEY").map {
            MessageDigest.getInstance("SHA-256").digest(it.toByteArray()).joinToString("") {byte->"%02x".format(byte)}
        }.orElse("unconfigured"))
        papoAbis.forEach {abi->outputs.file(nativeDirectory.map {it.file("$abi/libpapo_compose.so")})}
        commandLine(listOf("cargo","ndk")+papoAbis.flatMap {listOf("-t",it)}+
            listOf("--platform","31","-o",nativeDirectory.get().asFile.absolutePath,"build","--locked","-p","papo-android","--lib")+
            if(release) listOf("--release") else emptyList())
        doFirst {environment("ANDROID_NDK_HOME",androidComponents.sdkComponents.sdkDirectory.get().asFile.resolve("ndk/${android.ndkVersion}").absolutePath)}
    }
    val bindings=tasks.register<Exec>("generate${suffix}UniFFI") {
        dependsOn(native)
        workingDir(rootProject.file(".."))
        inputs.file(rootProject.file("../crates/papo-android/uniffi.toml"))
        inputs.file(nativeDirectory.map {it.file("${papoAbis.first()}/libpapo_compose.so")})
        outputs.dir(generatedDirectory)
        commandLine("cargo","run","--locked","-p","papo-android","--features","bindgen","--bin","papo-bindgen","--",
            "generate","--library",nativeDirectory.get().asFile.resolve("${papoAbis.first()}/libpapo_compose.so").absolutePath,
            "--language","kotlin",
            "--out-dir",generatedDirectory.get().asFile.absolutePath)
    }
    tasks.matching {it.name==if(release) "preReleaseBuild" else "preDebugBuild"}.configureEach {dependsOn(bindings)}
}
