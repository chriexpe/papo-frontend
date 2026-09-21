#!/usr/bin/env bash
# Constrói, instala e acompanha o Papo no Android.
#
#   scripts/android.sh              compila o .so e monta o APK de depuração
#   scripts/android.sh --install    monta e instala no aparelho conectado
#   scripts/android.sh --run        instala, abre e cola no logcat
#   scripts/android.sh --release    usa o perfil de release
#   scripts/android.sh --log        só acompanha o logcat
#
# O Gradle não compila o Rust: quem faz isso é o `cargo ndk`, que deixa o
# `libpapo.so` pronto em `app/src/main/jniLibs`. O Gradle só empacota.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"
APP_ID="io.github.chriexpe.Papo"
ABI="arm64-v8a"
# O piso de API do `cargo ndk` tem de bater com o `minSdk` do Gradle.
MIN_SDK=24

PROFILE=dev
DO_INSTALL=0
DO_RUN=0
DO_LOG_ONLY=0
KEEP_DEBUG=0

for arg in "$@"; do
    case "$arg" in
        --release) PROFILE=release ;;
        --install) DO_INSTALL=1 ;;
        --run) DO_INSTALL=1; DO_RUN=1 ;;
        --log) DO_LOG_ONLY=1 ;;
        --keep-debug) KEEP_DEBUG=1 ;;
        *) echo "argumento desconhecido: $arg" >&2; exit 2 ;;
    esac
done

# --- SDK e NDK -------------------------------------------------------------
: "${ANDROID_HOME:=$HOME/Android/Sdk}"
export ANDROID_HOME
if [ ! -d "$ANDROID_HOME" ]; then
    echo "SDK do Android não encontrado em $ANDROID_HOME" >&2
    exit 1
fi

# A versão do NDK é a mesma declarada no `app/build.gradle.kts`: o `.so` que
# o cargo produz e o que o Gradle empacota têm de vir do mesmo lugar.
NDK_VERSION=$(sed -n 's/.*ndkVersion *= *"\([^"]*\)".*/\1/p' android/app/build.gradle.kts)
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$NDK_VERSION"
if [ ! -d "$ANDROID_NDK_HOME" ]; then
    echo "NDK $NDK_VERSION não instalado. Instale com:" >&2
    echo "  \$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager --install 'ndk;$NDK_VERSION'" >&2
    exit 1
fi

# O Gradle não roda com JDK muito novo; o do Android Studio é o que se sabe
# que funciona. Quem já tiver um JAVA_HOME bom continua com o dele.
if [ -z "${JAVA_HOME:-}" ] && [ -d /opt/android-studio/jbr ]; then
    export JAVA_HOME=/opt/android-studio/jbr
fi

LOGCAT=("$ANDROID_HOME/platform-tools/adb" logcat -v color papo:V RustStdoutStderr:V \
        GameActivity:V AndroidRuntime:E libc:F DEBUG:V "*:S")

if [ "$DO_LOG_ONLY" = 1 ]; then
    exec "${LOGCAT[@]}"
fi

# --- Rust ------------------------------------------------------------------
JNI_LIBS="$ROOT/android/app/src/main/jniLibs"
CARGO_ARGS=(-t "$ABI" --platform "$MIN_SDK" -o "$JNI_LIBS" build --lib)
[ "$PROFILE" = release ] && CARGO_ARGS+=(--release)

echo "==> cargo ndk ($PROFILE, $ABI, API $MIN_SDK)"
cargo ndk "${CARGO_ARGS[@]}"

# Com os símbolos de depuração o `.so` passa de 380 MB e instalar demora
# minutos. `--strip-debug` derruba para menos de 30 MB e **mantém** a tabela
# de símbolos, então o backtrace nativo do logcat continua legível.
if [ "$KEEP_DEBUG" = 0 ]; then
    STRIP="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip"
    echo "==> llvm-strip --strip-debug"
    "$STRIP" --strip-debug "$JNI_LIBS/$ABI/libpapo.so"
fi
ls -la "$JNI_LIBS/$ABI/libpapo.so" | awk '{printf "    libpapo.so: %.1f MB\n", $5/1048576}'

# --- Gradle ----------------------------------------------------------------
if [ "$PROFILE" = release ]; then
    GRADLE_TASK=assembleRelease
    APK="$ROOT/android/app/build/outputs/apk/release/app-release-unsigned.apk"
else
    GRADLE_TASK=assembleDebug
    APK="$ROOT/android/app/build/outputs/apk/debug/app-debug.apk"
fi

echo "==> gradlew $GRADLE_TASK"
(cd android && ./gradlew --console=plain "$GRADLE_TASK")
ls -la "$APK" | awk '{printf "    apk: %.1f MB\n", $5/1048576}'

# --- Aparelho --------------------------------------------------------------
[ "$DO_INSTALL" = 1 ] || exit 0

ADB="$ANDROID_HOME/platform-tools/adb"
if [ -z "$("$ADB" devices | sed -n '2,$p' | grep -w device || true)" ]; then
    echo "nenhum aparelho conectado. Ligue o cabo e autorize a depuração USB." >&2
    exit 1
fi

echo "==> adb install"
"$ADB" install -r "$APK"

[ "$DO_RUN" = 1 ] || exit 0

echo "==> abrindo e acompanhando o logcat"
"$ADB" logcat -c
"$ADB" shell monkey -p "$APP_ID" -c android.intent.category.LAUNCHER 1 >/dev/null
exec "${LOGCAT[@]}"
