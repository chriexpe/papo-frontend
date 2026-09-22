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
APP_ID="io.github.chriexpe.papo"
ABI="arm64-v8a"
# A versão do SDK do GStreamer que casa com a da área de trabalho.
GST_VERSION="1.28.7"
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

JNI_LIBS="$ROOT/android/app/src/main/jniLibs"

# --- GStreamer -------------------------------------------------------------
# O GStreamer do Android vem de um SDK próprio, e vira um `.so` só: núcleo
# mais os plugins escolhidos no `CMakeLists.txt`, com um registro estático.
# Lá não existe `gst-plugin-scanner` nem plugin carregado em tempo de
# execução — o conjunto é decidido na ligação.
: "${GSTREAMER_ROOT_ANDROID:=$HOME/Android/gst}"
export GSTREAMER_ROOT_ANDROID
if [ ! -d "$GSTREAMER_ROOT_ANDROID/arm64" ]; then
    echo "SDK do GStreamer para Android não encontrado em $GSTREAMER_ROOT_ANDROID/arm64" >&2
    echo "Baixe o universal de https://gstreamer.freedesktop.org/data/pkg/android/$GST_VERSION/" >&2
    echo "e extraia ao menos 'arm64' ali." >&2
    exit 1
fi

GST_BUILD="$ROOT/android/gstreamer/build"
echo "==> cmake (libgstreamer_android.so)"
cmake -S "$ROOT/android/gstreamer" -B "$GST_BUILD" \
    -DCMAKE_TOOLCHAIN_FILE="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake" \
    -DANDROID_ABI="$ABI" \
    -DANDROID_PLATFORM="android-$MIN_SDK" \
    -DANDROID_STL=c++_shared \
    -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build "$GST_BUILD" -j"$(nproc)"

mkdir -p "$JNI_LIBS/$ABI"
cp "$GST_BUILD/libgstreamer_android.so" "$JNI_LIBS/$ABI/"
# O `.so` do GStreamer pede a libc++ compartilhada, que o NDK entrega.
cp "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" \
    "$JNI_LIBS/$ABI/"
ls -la "$JNI_LIBS/$ABI/libgstreamer_android.so" | awk '{printf "    libgstreamer_android.so: %.1f MB\n", $5/1048576}'

# O lado Rust liga contra esse `.so`. Dizer isso ao `system-deps` pela
# variável de ambiente evita montar um `pkg-config` cruzado inteiro só para
# ele descobrir o mesmo.
for dep in GSTREAMER_1_0 GSTREAMER_APP_1_0 GSTREAMER_VIDEO_1_0 \
           GSTREAMER_AUDIO_1_0 GSTREAMER_BASE_1_0 \
           GSTREAMER_WEBRTC_1_0 GSTREAMER_SDP_1_0 GSTREAMER_RTP_1_0 \
           GLIB_2_0 GOBJECT_2_0 GIO_2_0; do
    export "SYSTEM_DEPS_${dep}_NO_PKG_CONFIG=1"
    export "SYSTEM_DEPS_${dep}_LIB=gstreamer_android"
    export "SYSTEM_DEPS_${dep}_SEARCH_NATIVE=$JNI_LIBS/$ABI"
done

# --- Rust ------------------------------------------------------------------
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
