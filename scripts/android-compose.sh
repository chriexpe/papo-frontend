#!/usr/bin/env bash
# Builds only papo-core + its UniFFI adapter, then the native Compose UI.
set -euo pipefail
cd "$(dirname "$0")/.."
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/30.0.16248370"
export JAVA_HOME="${JAVA_HOME:-/opt/android-studio/jbr}"
abi="${PAPO_ANDROID_ABI:-arm64-v8a}"
install=0
run=0
test_app=0
variant=Debug
variant_dir=debug
for arg in "$@"; do
    case "$arg" in
        --x86_64) abi=x86_64 ;;
        --arm64) abi=arm64-v8a ;;
        --all-abis) abi=arm64-v8a,x86_64 ;;
        --install) install=1 ;;
        --run) install=1; run=1 ;;
        --test) install=1; test_app=1 ;;
        --release) variant=Release;variant_dir=release ;;
        *) echo "Usage: $0 [--arm64|--x86_64|--all-abis] [--install|--run|--test] [--release]" >&2; exit 2 ;;
    esac
done
case "$abi" in
    arm64-v8a) targets=(aarch64-linux-android) ;;
    x86_64) targets=(x86_64-linux-android) ;;
    arm64-v8a,x86_64) targets=(aarch64-linux-android x86_64-linux-android) ;;
    *) echo "Unsupported ABI: $abi" >&2; exit 2 ;;
esac
for target in "${targets[@]}"; do
    if ! rustup target list --installed | rg -q "^$target$"; then
        echo "Install Rust's target first: rustup target add $target" >&2
        exit 1
    fi
done
python3 scripts/prepare-winit.py
if [ "$variant" = Release ] && [ "$test_app" = 1 ]; then echo "Instrumentation tests use the debug variant" >&2;exit 2;fi
tasks=(":compose:assemble$variant")
if [ "$test_app" = 1 ]; then tasks+=(:compose:assembleDebugAndroidTest :compose:lintDebug); fi
./android/gradlew -p android "${tasks[@]}" "-PpapoAbis=$abi" --console=plain
apk="android/compose/build/outputs/apk/$variant_dir/compose-$variant_dir.apk"
if [ ! -f "$apk" ] && [ "$variant" = Release ]; then apk="android/compose/build/outputs/apk/release/compose-release-unsigned.apk"; fi
echo "APK: $PWD/$apk"
if [ "$install" = 1 ]; then "$ANDROID_HOME/platform-tools/adb" install -r "$apk"; fi
if [ "$test_app" = 1 ]; then
    "$ANDROID_HOME/platform-tools/adb" install -r android/compose/build/outputs/apk/androidTest/debug/compose-debug-androidTest.apk
    # UTP can report zero tests on a preview emulator. Run the suite directly
    # and require a nonzero JUnit pass count; adb's exit code alone is not enough.
    test_output=$("$ANDROID_HOME/platform-tools/adb" shell am instrument -w -r -e class \
        io.github.chriexpe.papo.compose.AllTests \
        io.github.chriexpe.papo.compose.test/androidx.test.runner.AndroidJUnitRunner)
    printf '%s\n' "$test_output"
    if ! printf '%s\n' "$test_output" | rg -q '^OK \([1-9][0-9]* tests?\)'; then
        echo "Device tests did not report a nonzero passing test count" >&2
        exit 1
    fi
fi
if [ "$run" = 1 ]; then
    app_id=io.github.chriexpe.papo.compose
    if [ "$variant" = Release ]; then app_id=io.github.chriexpe.papo; fi
    "$ANDROID_HOME/platform-tools/adb" shell am start -n "$app_id/io.github.chriexpe.papo.compose.MainActivity"
fi
