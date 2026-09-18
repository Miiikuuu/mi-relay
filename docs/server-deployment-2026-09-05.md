# VPS backend deployment — 2026-09-05

## Current state

The Rust backend and an independent HTTPS proxy are installed and enabled on
the test VPS (private host alias omitted). After the user confirmed having no domain, a publicly trusted IP
certificate was issued and the public HTTPS path was tested. The phone has **not**
yet been configured or tested against this deployment.

- Ubuntu 22.04 x86_64; release binary built locally against musl, static PIE,
  approximately 4.7 MiB. No compiler or build dependencies installed on the VPS.
- Binary: `/opt/mirelay/bin/mirelay-server`.
- SHA-256: `57ee872863ce6a9d78e16f6b7e732d03e37f31a8a4cefafe92ac349f45bc22e5`.
- Service: `mirelay-server.service`, dedicated non-login `mirelay` user,
  `127.0.0.1:8080` only, 100 MiB per-file limit, 256 MiB memory cap.
- Persistent state: `/var/lib/mirelay-server` (`0700`), SQLite (`0600`).
- Fresh 256-bit token: root-owned `/etc/mirelay/server.env` (`0600`). No SSH private
  key, Clash credentials, or previous QA token was used for application auth.
- The pre-existing SSH, Hysteria and Shadowsocks processes kept the same PIDs and
  activation timestamps. Their configuration and existing UFW rules were not
  modified. One destination-specific IPv4 TCP 443 allow rule was added for MiRelay.
- Caddy now listens on the VPS's public IPv4 TCP 443; UDP 443 still belongs to
  Hysteria. It uses `h1 h2`, a private Unix admin socket, and its own non-login
  `mirelay-proxy` account, state directory and 256 MiB memory cap.

## Verification

`cargo test --locked --lib --tests --no-default-features -- --test-threads=2`:
**85 passed**, 0 failed, 2 intentionally ignored entries (JNI integration and the
subprocess crash helper). This command does not cover the optional desktop UI or
Android device suites.

The VPS test used a temporary SSH tunnel binding locally to `127.0.0.1:18090` and
the new, otherwise unused server queue. No public cleartext listener was enabled.
The actual static release binary and systemd service handled these checks:

1. `/healthz` returned `ok`; missing and wrong tokens returned `401`.
2. An authenticated request missing the MiRelay API version returned `426`.
3. A 2,097,152-byte synthetic binary stopped after one 524,288-byte tus chunk.
4. `HEAD` reported offset **524288 before and after a service restart**. The sender
   resumed that same upload instead of creating a second delivery.
5. Binary and 64-byte UTF-8 text arrived at Linux with exact byte equality and
   matching SHA-256. Both were `acknowledged` and `not_applicable` to wallpaper.
6. A second sync received and acknowledged zero files; no duplicate deliveries.
7. Final server status was **pending 0 / acknowledged 2**. Reconciliation found no
   missing or corrupt referenced content; the acknowledged payloads were removed.

The temporary SSH forwarding session was closed after verification. The local
copy of the production token was removed; the protected server-side token remains.

Report: `target/deployment/smoke.bdgAtWAP/` (ignored by Git). The repeatable test is
[`deploy/smoke-test.sh`](../deploy/smoke-test.sh), checked with `bash -n` and
ShellCheck. It writes synthetic files only and removes its temporary auth header.

The first smoke-test attempt omitted the mandatory API version header and stopped
on the expected `426` before creating uploads. The script was corrected and rerun
successfully. An initial OpenSSL argument-order error likewise stopped installation
before service startup; the command was corrected without changing existing services.

The deployed systemd unit passed verification; systemd also reported an unrelated
existing `snapd.service` `RestartMode` warning, which was left untouched. Caddy
2.11.4 was downloaded locally with its SHA-256 checked against the official GitHub
release metadata; `caddy adapt` assertions and `caddy validate` passed for the
domain template. That initial check was configuration validation only; actual IP
certificate issuance and HTTPS verification are recorded below.

## HTTPS follow-up: no domain required

The deployed configuration is based on
[`deploy/Caddyfile.ip.example`](../deploy/Caddyfile.ip.example): explicit Let's
Encrypt ACME issuer, `shortlived` profile, TLS-ALPN-01, HTTP challenge/redirects
disabled, and binding only to the host's public IPv4 TCP 443. Public port 80 and
UDP 443 were not opened for MiRelay. Existing public HTTP allowance for 8080 was
left unchanged; the backend still binds only to loopback.

Local Caddy binary SHA-256 (also verified after copying to the VPS):
`b7105518e3ed1c0761f232e44fc09345535533c9cb0abf0e12809416c7ac64d9`.

Verification performed:

1. Staging issuance succeeded using TLS-ALPN-01. Normal curl rejected the staging
   chain (exit 60), as required. No application token was sent in this phase.
2. Production issuance succeeded. The final certificate's issuer is Let's Encrypt
   **YE2**, with an IP SAN matching this VPS; validity is September 5, 2026
   09:46:54 UTC through September 12, 2026 01:46:53 UTC.
3. Normal curl returned `/healthz` HTTP 200 over HTTP/2 with TLS verification 0.
   OpenSSL's `-verify_ip` and `-verify_return_error` checks passed with TLS 1.3.
   No extra CA was installed and certificate checks were not disabled.
4. A production proxy restart retained the same certificate fingerprint; HTTPS
   remained valid. Caddy received ACME renewal information and has automatic
   certificate maintenance enabled. A future scheduled renewal has **not** yet
   been observed over its full multi-day cycle.
5. Public HTTPS smoke test report `target/deployment/smoke.1vSyzieO/` passed:
   missing/wrong tokens 401, missing API version 426, 2 MiB binary pause at 512 KiB,
   backend restart with the same persisted offset, resume, UTF-8 text delivery,
   exact byte/hash checks, ACK and empty re-sync. No SSH tunnel was used for this
   upload/download path; SSH was used only to restart the backend deliberately.
6. Five subsequent HTTPS health probes all returned 200 with verification 0
   (roughly 0.8–1.0 seconds each). Service/kernel log checks found no warning/error,
   OOM or crash entries during the deployment/test window.

The first public smoke attempt (`target/deployment/smoke.GvHnU3o8/`) successfully
transferred, verified and acknowledged both files, but its final read-only queue
probe hit a connection timeout. The precise network cause was not isolated. The
GET/HEAD probes now have bounded transient retries (not TLS bypass or arbitrary
HTTP-error retries); the full repeat run above exited successfully. This does not
assert that the network will never time out.

Final server state: **pending 0 / acknowledged 6** (two earlier SSH-tunnel test
deliveries plus four public HTTPS test deliveries). Reconciliation found no
missing or corrupt references. All files in these checks were synthetic; no phone
files were accessed. Token copies/header files used locally were removed after
testing; the production token remains in its protected server-side file.

Certificate state lives under `/var/lib/mirelay-proxy` (`0700`), with the private
key mode `0600`. The proxy account cannot read `/etc/mirelay/server.env`. Neither
SSH keys nor application tokens are included in the Caddy configuration.

## Remaining before phone/public use

- Configure the Android app and repeat upload/resume/download/ACK tests through
  the phone's actual Clash routing; the desktop HTTPS test is not a device test.
- Observe scheduled certificate renewal and monitor expiry. TCP 443 must remain
  reachable for renewal; update configuration if the server's public IP changes.
- Establish backups, disk-space monitoring and a retention policy. Per-file and
  memory limits are not total disk quotas or protection against all resource abuse.
- This release still has one server-side device queue per instance; do not treat
  the single token as multi-user authorization or separate Folder isolation.
