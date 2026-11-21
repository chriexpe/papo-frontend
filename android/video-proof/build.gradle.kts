plugins { alias(libs.plugins.android.application) }

val nativeLibs = layout.buildDirectory.dir("proofJniLibs")
val fixtureSdk = providers.environmentVariable("GSTREAMER_ROOT_ANDROID")
    .orElse(providers.systemProperty("user.home").map { "$it/Android/gst" })

android {
    namespace = "io.github.chriexpe.papo.videoproof"
    compileSdk = 37
    ndkVersion = "30.0.16248370"
    defaultConfig {
        applicationId = "io.github.chriexpe.papo.videoproof"
        minSdk = 31
        targetSdk = 37
        ndk { abiFilters += setOf("arm64-v8a", "x86_64") }
    }
    sourceSets["debug"].jniLibs.directories.add(nativeLibs.get().asFile.path)
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

androidComponents.beforeVariants { variant ->
    variant.enable = variant.buildType == "debug"
}

val buildVideoProofNative = tasks.register<Exec>("buildVideoProofNative") {
    workingDir(rootProject.file(".."))
    // Let Cargo validate its own inputs and incremental cache on every invocation.
    outputs.upToDateWhen { false }
    outputs.dir(nativeLibs)
    commandLine("bash", "scripts/papo-android-video-proof.sh", nativeLibs.get().asFile.absolutePath)
    doFirst {
        environment("GSTREAMER_ROOT_ANDROID", fixtureSdk.get())
        val sdk = androidComponents.sdkComponents.sdkDirectory.get().asFile
        environment("ANDROID_NDK_HOME", sdk.resolve("ndk/${android.ndkVersion}").absolutePath)
    }
}

tasks.named("preBuild") { dependsOn(buildVideoProofNative) }
