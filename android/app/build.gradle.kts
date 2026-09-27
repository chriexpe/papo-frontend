import java.util.Properties

val cargoManifest = rootProject.file("../Cargo.toml")
val cargoVersion = cargoManifest.readLines()
    .first { it.trimStart().startsWith("version = ") }
    .substringAfter("=")
    .trim()
    .trim('"')

val versionParts = cargoVersion.substringBefore('-').split('.').map { it.toInt() }
require(versionParts.size == 3) { "Cargo package version must be MAJOR.MINOR.PATCH" }
val papoVersionCode =
    versionParts[0] * 1_000_000 +
    versionParts[1] * 1_000 +
    versionParts[2]
require(versionParts[1] < 1_000 && versionParts[2] < 1_000) {
    "Android versionCode encoding supports MINOR/PATCH values below 1000"
}
require(papoVersionCode in 1..2_100_000_000) { "Android versionCode out of range" }

plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "io.github.chriexpe.papo"
    compileSdk = 37
    // O NDK que compila o Rust é o mesmo que o Gradle empacota: versões
    // diferentes aqui e no `cargo ndk` rendem `.so` que não casam.
    ndkVersion = "30.0.16248370"

    defaultConfig {
        // Minúsculo, que é a convenção do Android — e por isso diferente do
        // `APP_ID` do Flatpak, que capitaliza a última parte pela convenção
        // do AppStream. As duas não cabem na mesma string.
        //
        // Este nome é para sempre: trocá-lo depois de alguém instalar não é
        // atualização, é outro aplicativo — instalação separada, sessão e
        // ajustes perdidos.
        applicationId = "io.github.chriexpe.papo"
        minSdk = 31
        targetSdk = 37
        // Cargo.toml is the single version source shared with GitHub tags.
        // Android additionally needs a monotonically increasing integer.
        versionCode = papoVersionCode
        versionName = cargoVersion

        // Por enquanto só o celular de verdade (arm64). O emulador x86_64
        // entra quando alguém precisar dele.
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    // O `.so` não é compilado pelo Gradle: quem faz isso é o `cargo ndk`,
    // que o deixa pronto em `src/main/jniLibs` — o lugar padrão, por isso
    // não há nada a configurar aqui. Nada de `externalNativeBuild` nem de
    // prefab: o `android-activity` traz a própria camada de cola nativa, e
    // a do GameActivity por cima dela quebraria as duas.

    buildTypes {
        debug {
            isJniDebuggable = true
        }
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // O Rust entrega o `libpapo.so`; o `cargo ndk` também copia os cdylibs
    // dos kits do Turso, que existem só para a ABI C e não entram no
    // `DT_NEEDED` de ninguém. Empacotá-los inflaria o APK sem uso. O único
    // código nativo do Turso que interessa é o `simsimd`, compilado
    // estaticamente para dentro do `libpapo.so`.
    packaging {
        jniLibs {
            excludes += setOf(
                "**/libturso_sdk_kit-*.so",
                "**/libturso_sync_sdk_kit-*.so",
            )
        }
    }
}

dependencies {
    implementation(libs.games.activity)
    implementation(libs.appcompat)
    implementation(libs.work.runtime)
}
