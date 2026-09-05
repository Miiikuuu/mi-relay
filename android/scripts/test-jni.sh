#!/usr/bin/env bash
set -euo pipefail
android_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_BUILD_JOBS=1 cargo build --locked --manifest-path "$android_root/native/Cargo.toml"
mkdir -p "$android_root/.local/jni-tests"
javac --release 17 -d "$android_root/.local/jni-tests" \
  "$android_root/app/src/main/java/io/mirelay/android/NativeBridge.java" "$android_root/native/tests/NativeSmoke.java"
java --enable-native-access=ALL-UNNAMED -Djava.library.path="$android_root/native/target/debug" \
  -cp "$android_root/.local/jni-tests" NativeSmoke "$@"
