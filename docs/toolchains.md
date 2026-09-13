# Rust toolchains and minimum-version validation

The minimum supported Rust version (MSRV) for the **locked Linux root build and
Android native build is 1.88**. Both package manifests declare it. This is not a
claim that every platform accepted by Cargo is supported.

Development and candidate builds use **1.98.1**, pinned in the root
`rust-toolchain.toml`; rustup applies it to `android/native` too. This does not
change the user's global default. Rust 1.98.1 was the current stable reported by
the official rustup channel check during this audit; the pin does not silently
follow future stable releases. Updating it requires another validation run.

The old 1.85 declaration was incorrect. The source uses
[let chains, stabilized in Rust 1.88](https://blog.rust-lang.org/2025/06/26/Rust-1.88.0/).
In addition, `android/native/Cargo.lock` selects ICU 2.3 packages whose manifests
require 1.88. The root lockfile selects a different dependency graph: inspecting
only that graph or compiling only with the newest compiler is insufficient.
Neither lockfile is regenerated to satisfy this correction.

## Reproduce the build matrix

Use Linux x86_64, Python 3.11+ (with `tarfile` data-filter support), Git, rustup,
a C/C++ toolchain, `pkg-config`, GTK 4.6+ / libadwaita development packages, GLib's
resource compiler and Android NDK **27.2.12479018 (r27c)**. The Python runner uses
offline Cargo builds; populate the Cargo crate cache beforehand if necessary.
The Android SDK license/bootstrap remains an explicit separate action.

Run these commands from the repository root so rustup discovers the pin. Explicit
`cargo +VERSION` and `RUSTUP_TOOLCHAIN` overrides take precedence; record them when
used. Install explicit toolchains (not a global `rustup default` change):

```bash
rustup toolchain install 1.88.0 --profile minimal \
  --target aarch64-linux-android --target x86_64-linux-android
rustup toolchain install 1.98.1 --profile minimal --component rustfmt --component clippy \
  --target aarch64-linux-android --target x86_64-linux-android

cargo +1.88.0 fetch --locked
cargo +1.88.0 fetch --locked --manifest-path android/native/Cargo.toml
python3 scripts/validate-toolchains.py --revision HEAD
```

The runner creates a fresh `git archive` source export and empty per-toolchain
build directories under private, ignored `target/toolchain-validation-*`. It
does not copy an existing target directory, device files or untracked user data.
It performs **eight builds**:

| Compiler | Linux default | Linux desktop | Android ARM64 JNI | Android x86_64 JNI |
| --- | --- | --- | --- | --- |
| 1.88.0 | `build --all-targets` | `build --all-targets --features desktop` | `build --release --target aarch64-linux-android` | `build --release --target x86_64-linux-android` |
| 1.98.1 | Same | Same | Same | Same |

All use `--locked --offline`; Android uses its own manifest/lock and the same
API 26 NDK linkers / 16 KiB linker flag as `android/scripts/build.sh`. Linux
builds compile test and example targets but **do not run tests**. Desktop shares
the compiler's default-build cache, while compiler versions have separate empty
starting directories. Cached dependency sources are shared, not compiled outputs.

`results.json` records the base commit, optional metadata overlays, source-index
hash, compiler/Cargo identities, environment, commands, exit codes, elapsed times,
log hashes and binary hashes. `source-sha256.json` identifies every input file.
Logs are kept on failure. Both lockfiles are checked for changes. The runner's
own checks use fake compilers and must not be counted as successful real builds:

```bash
python3 -m unittest discover -s scripts -p 'test_*.py' -v
```

For pre-commit A1 work, `--working-metadata` overlays **only** `Cargo.toml`,
`android/native/Cargo.toml` and `rust-toolchain.toml` from the working tree. It
records their hashes rather than claiming a pristine committed candidate.
Uncommitted source changes are not copied. For A3 release acceptance, commit the
intended changes, pass the frozen SHA and omit this option.

## Scope

This is build/toolchain validation, not APK signing, Android installation, runtime
compatibility, 16 KiB device acceptance, a dependency vulnerability scan or a
release certification. Final candidate testing must still cover GTK, JNI,
emulator/phone, installer, release binaries and the unresolved recovery-lock
investigation separately. See [release validation status](../RELEASE_VALIDATION.md).
