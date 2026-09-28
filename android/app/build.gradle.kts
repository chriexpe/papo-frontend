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

val releaseKeystore = System.getenv("PAPO_ANDROID_KEYSTORE")?.takeIf { it.isNotBlank() }
val releaseStorePassword = System.getenv("PAPO_ANDROID_STORE_PASSWORD")
val releaseKeyAlias = System.getenv("PAPO_ANDROID_KEY_ALIAS")
val releaseKeyPassword = System.getenv("PAPO_ANDROID_KEY_PASSWORD")

// Configuração do app Android no projeto Firebase (os campos do
// google-services.json). Vem de variável de ambiente ou de propriedade do
// Gradle (`~/.gradle/gradle.properties`), nunca do repositório: um fork não
// herda o projeto de ninguém. Sem os quatro valores o APK sai sem FCM e as
// notificações em segundo plano ficam com a reconciliação periódica.
fun firebaseValue(name: String): String? =
    (System.getenv(name) ?: providers.gradleProperty(name).orNull)?.takeIf { it.isNotBlank() }

val firebaseConfig = mapOf(
    "google_app_id" to firebaseValue("PAPO_FIREBASE_APP_ID"),
    "google_api_key" to firebaseValue("PAPO_FIREBASE_API_KEY"),
    "project_id" to firebaseValue("PAPO_FIREBASE_PROJECT_ID"),
    "gcm_defaultSenderId" to firebaseValue("PAPO_FIREBASE_SENDER_ID"),
)
val firebaseConfigured = firebaseConfig.values.all { it != null }
if (!firebaseConfigured && firebaseConfig.values.any { it != null }) {
    logger.warn("Papo: configuração do Firebase incompleta; o APK sai sem FCM")
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

        // Os mesmos recursos que o plugin google-services geraria: com eles o
        // FirebaseInitProvider inicializa o Firebase sozinho na abertura.
        if (firebaseConfigured) {
            firebaseConfig.forEach { (name, value) -> resValue("string", name, value!!) }
        }
    }

    buildFeatures {
        resValues = true
    }

    // O `.so` não é compilado pelo Gradle: quem faz isso é o `cargo ndk`,
    // que o deixa pronto em `src/main/jniLibs` — o lugar padrão, por isso
    // não há nada a configurar aqui. Nada de `externalNativeBuild` nem de
    // prefab: o `android-activity` traz a própria camada de cola nativa, e
    // a do GameActivity por cima dela quebraria as duas.

    signingConfigs {
        if (releaseKeystore != null) {
            create("release") {
                storeFile = file(releaseKeystore)
                storePassword = releaseStorePassword
                    ?: error("PAPO_ANDROID_STORE_PASSWORD is required for signed releases")
                keyAlias = releaseKeyAlias
                    ?: error("PAPO_ANDROID_KEY_ALIAS is required for signed releases")
                keyPassword = releaseKeyPassword
                    ?: error("PAPO_ANDROID_KEY_PASSWORD is required for signed releases")
            }
        }
    }

    buildTypes {
        debug {
            isJniDebuggable = true
        }
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (releaseKeystore != null) {
                signingConfig = signingConfigs.getByName("release")
            }
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
    implementation(platform(libs.firebase.bom))
    implementation(libs.firebase.messaging)
}
