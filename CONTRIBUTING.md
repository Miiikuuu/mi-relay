# Contributing

MiRelay is an experimental Android → relay → Linux file-delivery project.
Discuss protocol changes, destructive operations and large UI changes before
implementing them. Keep existing files, pairing state and compatibility intact.

Use synthetic media, a local relay, and isolated configuration/state directories
for development. Never point an automated test at a personal Folder or production
relay. Do not commit screenshots of private media, credentials or test databases.

## Local checks

Follow [toolchain setup](docs/toolchains.md) and [Android setup](android/README.md).
For a Rust/desktop change:

```sh
cargo fmt --all -- --check
cargo test --locked --features desktop --all-targets
cargo clippy --locked --all-targets --all-features -- -D warnings
python3 -m unittest discover -s scripts -p 'test_*.py'
python3 -m unittest discover -s deploy -p 'test_*.py'
```

For Android changes, also run the appropriate JVM tests and lint. Real device,
graphical, background and deployment checks are separate gates; do not describe
ignored or unrun tests as passing. See [CI boundaries](docs/ci.md).

Keep user-facing copy in English and include regression tests for fixes. Report
the exact tests run, their results and any remaining limits in your pull request.
Do not upload Android release signing keys or passwords to CI.

Original code/documentation contributions are accepted under [MIT](LICENSE).
Brand assets and third-party icons retain their [separate terms](NOTICE.md).
For vulnerabilities, follow [SECURITY.md](SECURITY.md), not a public bug report.
