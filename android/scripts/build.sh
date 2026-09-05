#!/usr/bin/env bash
set -euo pipefail
android_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
tools_dir="$android_root/.local"
sdk_dir="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$tools_dir/sdk}}"
ndk_dir="$sdk_dir/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64"
if [[ ! -x "$tools_dir/jdk/bin/java" || ! -x "$ndk_dir/bin/clang" ]]; then
  echo 'Missing toolchain. Read android/README.md and run the explicit bootstrap step.' >&2
  exit 2
fi
verifier_manifest="$(cargo metadata --locked --format-version 1 --filter-platform aarch64-linux-android --manifest-path "$android_root/native/Cargo.toml" | jq -er '.packages[] | select(.name == "rustls-platform-verifier-android") | .manifest_path')"
verifier_aar="$(find "$(dirname -- "$verifier_manifest")/maven" -name '*.aar' -print -quit)"
test -n "$verifier_aar"
cp "$verifier_aar" "$tools_dir/rustls-platform-verifier.aar"
for pair in 'aarch64-linux-android arm64-v8a aarch64-linux-android26' 'x86_64-linux-android x86_64 x86_64-linux-android26'; do
  read -r rust_target abi clang_target <<< "$pair"
  linker_key="CARGO_TARGET_${rust_target^^}_LINKER"
  linker_key="${linker_key//-/_}"
  rust_target_key="${rust_target//-/_}"
  env "$linker_key=$ndk_dir/bin/$clang_target-clang" \
    "CC_$rust_target_key=$ndk_dir/bin/$clang_target-clang" \
    "AR_$rust_target_key=$ndk_dir/bin/llvm-ar" \
    CARGO_BUILD_JOBS=1 RUSTFLAGS='-C link-arg=-Wl,-z,max-page-size=16384' \
    cargo build --locked --manifest-path "$android_root/native/Cargo.toml" --release --target "$rust_target"
  mkdir -p "$android_root/app/src/main/jniLibs/$abi"
  cp "$android_root/native/target/$rust_target/release/libmirelay_android.so" "$android_root/app/src/main/jniLibs/$abi/"
done
env JAVA_HOME="$tools_dir/jdk" ANDROID_HOME="$sdk_dir" ANDROID_USER_HOME="$tools_dir/android-user" \
  GRADLE_USER_HOME="$tools_dir/gradle-home" \
  "$tools_dir/gradle-8.13/bin/gradle" --project-dir "$android_root" --no-daemon --console=plain \
  "${@:-:app:assembleDebug}"
