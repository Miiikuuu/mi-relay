#!/usr/bin/env bash
# One repository-owned emulator; never selects a USB phone or wipes an AVD.
set -euo pipefail
android_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
tools_dir="$android_root/.local"
export JAVA_HOME="$tools_dir/jdk"
export ANDROID_HOME="$tools_dir/sdk"
export ANDROID_USER_HOME="$tools_dir/android-user"
export ANDROID_AVD_HOME="$tools_dir/avd"
avd_name=MiRelay_API36_QA
serial=emulator-5580
adb_bin="$ANDROID_HOME/platform-tools/adb"
emulator_bin="$ANDROID_HOME/emulator/emulator"
owned_device() {
  local actual
  actual="$("$adb_bin" -s "$serial" emu avd name | tr -d '\r' | head -n 1)"
  [[ "$actual" == "$avd_name" ]] || { echo "Refusing unrelated device: $actual" >&2; exit 2; }
}
case "${1:-help}" in
  setup)
    [[ "${2:-}" == --accept-sdk-license ]] || { echo 'Read the Android SDK license; setup requires --accept-sdk-license.' >&2; exit 2; }
    "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --sdk_root="$ANDROID_HOME" 'emulator' 'system-images;android-36;default;x86_64'
    mkdir -p "$ANDROID_AVD_HOME"
    if [[ ! -e "$ANDROID_AVD_HOME/$avd_name.ini" ]]; then
      printf 'no\n' | "$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager" create avd \
        --name "$avd_name" --path "$ANDROID_AVD_HOME/$avd_name.avd" \
        --package 'system-images;android-36;default;x86_64' --device pixel_2
    fi
    "$emulator_bin" -accel-check
    ;;
  start)
    [[ -f "$ANDROID_AVD_HOME/$avd_name.ini" ]] || { echo 'Run setup first.' >&2; exit 2; }
    graphics="${MIRELAY_EMULATOR_GPU:-swiftshader}"
    case "$graphics" in
      swiftshader|swangle|lavapipe) ;;
      *) echo 'MIRELAY_EMULATOR_GPU must be swiftshader, swangle, or lavapipe.' >&2; exit 2 ;;
    esac
    "$emulator_bin" -accel-check
    "$adb_bin" start-server
    if "$adb_bin" -s "$serial" get-state >/dev/null 2>&1; then
      owned_device; echo 'MiRelay emulator is already running.'; exit 0
    fi
    window=(-no-window)
    [[ "${2:-}" != --window ]] || window=()
    exec nice -n 10 "$emulator_bin" -avd "$avd_name" -port 5580 \
      -memory 2560 -cores 2 -gpu "$graphics" -skin 720x1280 \
      -no-audio -no-boot-anim -no-snapshot -no-metrics \
      -camera-back none -camera-front none "${window[@]}"
    ;;
  check)
    "$emulator_bin" -accel-check
    owned_device
    [[ "$("$adb_bin" -s "$serial" shell getprop sys.boot_completed | tr -d '\r')" == 1 ]]
    "$adb_bin" -s "$serial" shell getprop ro.build.version.release
    "$adb_bin" -s "$serial" shell getprop ro.build.version.sdk
    "$adb_bin" -s "$serial" shell getprop ro.product.cpu.abi
    "$adb_bin" -s "$serial" shell getconf PAGE_SIZE
    "$adb_bin" -s "$serial" shell wm size
    "$adb_bin" -s "$serial" shell df -h /data
    echo 'Dedicated emulator is booted and reachable.'
    ;;
  adb)
    shift; owned_device; exec "$adb_bin" -s "$serial" "$@"
    ;;
  stop)
    owned_device; "$adb_bin" -s "$serial" emu kill
    ;;
  *) echo 'Usage: bash android/scripts/emulator.sh setup --accept-sdk-license | start [--window] | check | adb ARGS... | stop' ;;
esac
