#!/usr/bin/env bash
# Opt-in, repository-local toolchain. Never changes shell startup files.
set -euo pipefail
android_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
tools_dir="$android_root/.local"
if [[ "${1:-}" != "--accept-sdk-license" ]]; then
  echo 'Read https://developer.android.com/studio#terms-and-conditions first.'
  echo 'To accept the Android SDK licenses and download the local toolchain, pass --accept-sdk-license.'
  exit 2
fi
mkdir -p "$tools_dir/downloads" "$tools_dir/sdk" "$tools_dir/android-user"
fetch() {
  local direct=()
  if [[ "${MIRELAY_ANDROID_DIRECT_DOWNLOADS:-0}" == 1 ]]; then direct=(--noproxy dl.google.com); fi
  echo "Downloading $(basename -- "$2")"
  curl --fail --silent --show-error --location --retry 3 --connect-timeout 20 "${direct[@]}" --output "$2" "$1"
}
verify() { printf '%s  %s\n' "$1" "$2" | sha256sum --check --status; }
if [[ ! -x "$tools_dir/jdk/bin/java" ]]; then
  fetch 'https://api.adoptium.net/v3/assets/latest/17/hotspot?architecture=x64&image_type=jdk&os=linux&vendor=eclipse' "$tools_dir/downloads/jdk.json"
  jdk_url="$(jq -er '.[0].binary.package.link' "$tools_dir/downloads/jdk.json")"
  jdk_sha="$(jq -er '.[0].binary.package.checksum' "$tools_dir/downloads/jdk.json")"
  fetch "$jdk_url" "$tools_dir/downloads/jdk.tar.gz"
  verify "$jdk_sha" "$tools_dir/downloads/jdk.tar.gz"
  mkdir -p "$tools_dir/jdk"
  tar -xzf "$tools_dir/downloads/jdk.tar.gz" --strip-components=1 -C "$tools_dir/jdk"
fi
if [[ ! -x "$tools_dir/gradle-8.13/bin/gradle" ]]; then
  fetch 'https://services.gradle.org/distributions/gradle-8.13-bin.zip.sha256' "$tools_dir/downloads/gradle.sha256"
  fetch 'https://services.gradle.org/distributions/gradle-8.13-bin.zip' "$tools_dir/downloads/gradle.zip"
  verify "$(tr -d '\r\n' < "$tools_dir/downloads/gradle.sha256")" "$tools_dir/downloads/gradle.zip"
  unzip -q -o "$tools_dir/downloads/gradle.zip" -d "$tools_dir"
fi
if [[ ! -x "$tools_dir/sdk/cmdline-tools/latest/bin/sdkmanager" ]]; then
  fetch 'https://dl.google.com/android/repository/commandlinetools-linux-15859902_latest.zip' "$tools_dir/downloads/sdk-tools.zip"
  verify '4e4c464f145a7512b57d088ac6c278c03c9eea610886b35a5e0804e74eedf583' "$tools_dir/downloads/sdk-tools.zip"
  mkdir -p "$tools_dir/sdk/cmdline-tools"
  unzip -q -o "$tools_dir/downloads/sdk-tools.zip" -d "$tools_dir/sdk/cmdline-tools"
  mv "$tools_dir/sdk/cmdline-tools/cmdline-tools" "$tools_dir/sdk/cmdline-tools/latest"
fi
sdk_manager="$tools_dir/sdk/cmdline-tools/latest/bin/sdkmanager"
# The explicit flag above is required; never accept licenses implicitly.
set +o pipefail
yes | env JAVA_HOME="$tools_dir/jdk" ANDROID_USER_HOME="$tools_dir/android-user" \
  "$sdk_manager" --sdk_root="$tools_dir/sdk" 'platforms;android-36' 'build-tools;35.0.0' 'platform-tools' 'ndk;27.2.12479018'
license_status="${PIPESTATUS[1]}"
set -o pipefail
[[ "$license_status" == 0 ]]
rustup target add aarch64-linux-android x86_64-linux-android
echo 'Local Android toolchain is ready. Build with bash android/scripts/build.sh.'
