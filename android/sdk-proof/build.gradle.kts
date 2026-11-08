plugins { alias(libs.plugins.android.application) }

val nativeDirectory = layout.buildDirectory.dir("rustJniLibs").get().asFile
val generatedDirectory = layout.buildDirectory.dir("generated/uniffi").get().asFile

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
    // AGP's built-in Kotlin source sets require a directory, not a Provider.
    sourceSets["main"].kotlin.srcDir(generatedDirectory)
    sourceSets["main"].jniLibs.srcDir(nativeDirectory)
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

val generateProofBindings =
    tasks.register<Exec>("generateProofBindings") {
        workingDir(rootProject.file(".."))
        inputs.files(rootProject.file("../Cargo.toml"), rootProject.file("../Cargo.lock"))
        inputs.files(
            fileTree(rootProject.file("../crates/sdk")) {
                include("**/Cargo.toml", "**/src/**")
            }
        )
        inputs.file(rootProject.file("../scripts/papo-sdk-android-proof.sh"))
        outputs.dir(nativeDirectory)
        outputs.dir(generatedDirectory)
        commandLine(
            "bash",
            "scripts/papo-sdk-android-proof.sh",
            nativeDirectory.absolutePath,
            generatedDirectory.absolutePath,
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

tasks.named("preBuild") { dependsOn(generateProofBindings) }

dependencies {
    implementation("net.java.dev.jna:jna:5.18.1@aar")
    androidTestImplementation(platform("androidx.compose:compose-bom:2026.09.00"))
    androidTestImplementation("androidx.compose.runtime:runtime")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}

val proofFormatter by configurations.creating

dependencies {
    proofFormatter("com.facebook:ktfmt:0.64:with-dependencies") { isTransitive = false }
}

val proofKotlinSources = fileTree("src") { include("**/*.kt") } + files("build.gradle.kts")

listOf("formatSdkProof", "checkSdkProofFormatting").forEach { taskName ->
    tasks.register<JavaExec>(taskName) {
        classpath = proofFormatter
        mainClass.set("com.facebook.ktfmt.cli.Main")
        inputs.files(proofKotlinSources)
        doFirst {
            args =
                listOf("--kotlinlang-style") +
                    (if (taskName.startsWith("check")) listOf("--dry-run", "--set-exit-if-changed")
                    else emptyList()) +
                    proofKotlinSources.files.sortedBy { it.path }.map { it.absolutePath }
        }
    }
}
