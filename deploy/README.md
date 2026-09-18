# Independent HTTPS deployment

The templates and `setup.py` helper must not overwrite another installation or
proxy configuration. Install/upgrade defaults to a non-mutating plan; `--apply`
is required to change this host. See [Folder pairing](../docs/folder-pairing.md).

## Assisted install, upgrade and check

Requires Linux/systemd, Python 3.9+, curl, iproute2, useradd/getent, and compatible
release binaries. Build the backend from the repository root on a development
machine using the [pinned Rust toolchain](../docs/toolchains.md) (MSRV 1.88) with
`cargo build --locked --release --bin mirelay-server` and copy the correct
architecture/libc build to the VPS. Obtain a compatible Caddy release from its
official distribution. Independently verify expected artifact SHA-256 values.

Copy this `deploy` directory and the binaries to the VPS, then review the plan:

```bash
python3 deploy/setup.py install --address relay.example.com \
  --server-binary /absolute/path/mirelay-server --server-sha256 EXPECTED_SHA256 \
  --caddy-binary /absolute/path/caddy --caddy-sha256 EXPECTED_SHA256
```

Repeat as root with `--apply` only after review. The helper checks TCP conflicts,
refuses existing MiRelay paths/accounts, validates staged Caddy configuration, and
creates distinct root-only admin/legacy tokens. It never modifies firewall rules.
Allow inbound **TCP 443** without changing SSH/other proxy rules; wait for certificate
issuance and run:

```bash
python3 deploy/setup.py check --address relay.example.com
```

A public IPv4 address assigned directly to this host can replace the domain. NAT
frontends are not supported by the IP template. Domain mode requires correct DNS.
UDP 443 and a public HTTP backend are not needed. Health checks validate today's
trusted certificate, not future renewal.

For an existing installation, review this plan, then repeat as root with `--apply`:

```bash
python3 deploy/setup.py upgrade \
  --server-binary /absolute/path/new-mirelay-server --server-sha256 EXPECTED_SHA256
```

Upgrade stops only the backend and creates a private full data/binary/env backup
under `/var/backups/mirelay/upgrade-*`. Existing tokens are preserved; an admin token
is added only if absent. Caddy, certificates and firewall remain unchanged. Plan a
maintenance window. Once the new backend accepts data, blind database rollback can
lose deliveries, so it is not automatic. Keep the reported backup and inspect any
partial-install/upgrade failure before retrying.

Read `MIRELAY_ADMIN_TOKEN` privately from `/etc/mirelay/server.env` and use it only
in Linux → Pairing → Administrator credential. Never send it to Android or paste
the env file in chat/logs. Normal Folder creation then uses the authenticated API
without SSH.

Guard tests: `python3 -m unittest discover -s deploy -p 'test_*.py' -v`.
An upgrade/recovery rehearsal can run without host root using bubblewrap:

```bash
python3 deploy/rehearse-upgrade.py \
  --old-binary /absolute/path/previous-mirelay-server \
  --new-binary /absolute/path/new-mirelay-server \
  --old-schema 4 --new-schema 5
```

This uses disposable user/mount/PID/network namespaces, synthetic files, real
binaries/SQLite/HTTP and the actual `upgrade --apply` filesystem path. A child
process controller replaces systemctl; it is not a systemd or fresh-install test.
It covers failed checksum/backup/fsync, migration, interrupted tus retention,
pairing, post-start health failure without database rollback, and isolated restore.
Backup directories and their parent entries are fsynced before replacing live files.

The current rehearsal defaults to schema **4 → 5** (coordinated clean exit). It seeds both
ready and unclaimed Folders plus a partially uploaded scoped tus file before the
upgrade, compares every pre-existing column and row exactly, preserves both credentials,
and verifies that old Folders are not initialized as directory sync implicitly.
It also checks a new directory inventory, a Unicode path, restart/resume, legacy
queue isolation, receipt role/hash checks, and shared-content retention after ACK.
The only added column allowed on an old Folder is `disconnected=0`; migration
must not close an existing connection. The rehearsal also checks both-device
revocation, idempotent retries, rejection after restart and other-Folder isolation.
Its receipts are synthetic receiver requests after byte verification, not evidence
of a physical phone or GTK application completing a transfer.

Schema 5 must introduce an empty exit-receipt table without retiring old Folders.
The rehearsal additionally verifies clean-exit authorization, scoped payload/tus
cleanup, shared-content preservation, both-device revocation, restart persistence
and idempotent acknowledgements. It never upgrades the production server.

Use `--old-schema 1 --new-schema 2` to repeat the earlier pairing-only migration,
`--old-schema 2 --new-schema 3` for directory sync,
`--old-schema 3 --new-schema 4` for disconnection, or `--old-schema 1 --new-schema 5`
for a direct upgrade from the original server. Supply matching binaries.
Expected schemas are explicit assertions, not inferred from whichever binary
happens to be present. Inputs are copied and hash-checked before being mounted
read-only into the namespace; binaries stored under `/tmp` are supported. The
script must not be run with Python `-O`, which would disable its assertions.

The existing Ubuntu 22.04 VPS backend upgrade was explicitly authorized and
accepted on September 5, 2026; see [upgrade evidence](../docs/pairing-deployment-2026-09-05.md).
The schema-4 upgrade was accepted on September 12; see
[disconnection rollout evidence](../docs/server-disconnection-2026-09-12.md).
The schema-5 clean-exit upgrade and subsequent early-ACK response correction were
accepted on September 18; see [clean-exit rollout evidence](../docs/server-clean-exit-2026-09-18.md).
The **fresh root installer remains untested end-to-end**. Other hosts/configurations
still require their own preflight and backup/recovery acceptance.

## Layout

- `/opt/mirelay/bin/mirelay-server`: root-owned release binary, mode `0755`.
- `/etc/mirelay/server.env`: root-owned, mode `0600`; contains a freshly generated
  256-bit `MIRELAY_SERVER_TOKEN`. systemd reads this file before dropping privileges.
- `/var/lib/mirelay-server`: owned by the dedicated `mirelay` system account,
  created by systemd with mode `0700`.
- `/opt/mirelay/bin/caddy`: separately verified official Caddy binary.
- `/etc/mirelay/Caddyfile`: non-secret configuration, readable by `mirelay-proxy`.
- `/var/lib/mirelay-proxy`: certificates and ACME state; owned by the separate
  `mirelay-proxy` system account, mode `0700`. Do not discard on service updates.

Neither account needs a login shell. MiRelay's HTTP service binds **only** to
`127.0.0.1:8080`. Do not pass `--allow-public-http` in production.

## HTTPS preconditions

Confirm the chosen domain and its A/AAAA records before replacing
`relay.example.com` in `Caddyfile.example`. Publicly trusted certificates and
their automatic renewal require the appropriate public TCP ports to be reachable.
Never use a real bearer token over public HTTP or disable certificate validation.

### Without a domain

For a public IPv4 address assigned directly to the VPS, use `Caddyfile.ip.example`
instead. Replace both example IP addresses with that address. Caddy 2.11.4 supports
Let's Encrypt IP certificates with the explicit `shortlived` ACME profile. These
certificates last about six days; keep the proxy enabled, its state persistent,
the clock synchronized, and public TCP 443 reachable for automatic renewal.

The IP template disables both HTTP redirects and the HTTP-01 challenge. It uses
TLS-ALPN-01 on **TCP 443 only**; no public port 80 or UDP 443 is needed. It must not
use `tls internal`, `on_demand`, or a client certificate-validation bypass.

Test issuance against the Let's Encrypt staging directory first, without sending
any application token. An ordinary client **must reject** the staging certificate.
Then validate the production config and restart only `mirelay-proxy.service` to
avoid retaining the staging certificate in the running cache. Verify the final
certificate chain **and IP SAN** with a normal client before sending real tokens.

This avoids a domain purchase, but the address is less memorable and changes if
the VPS's public IP changes. Auto-renewal configuration is not evidence that a
future renewal has already succeeded: expiry and service health need monitoring.

Check existing TCP **and UDP** listeners and firewall rules before deployment.
These templates deliberately enable only HTTP/1.1 and HTTP/2: **UDP 443 stays
available to Hysteria**. Keep existing SSH, Shadowsocks and Clash configuration
unchanged. Add only the required TCP firewall rules; never reset the firewall.

The Caddy admin endpoint is a private Unix socket. No public admin port or access
logging is enabled. Both services have separate state, accounts and memory caps.
The caps are starting values for a small, personal server, not a load-test result.
They do not provide a disk quota: unfinished uploads and retained delivery metadata
still need disk-space monitoring and a deliberate retention/backup policy.

Before enabling services, run `systemd-analyze verify` on the installed units and
`caddy validate --config /etc/mirelay/Caddyfile --adapter caddyfile` with the proxy's
configured identity/environment. Ensure its runtime and state directories exist.

## Acceptance checks

1. Backend health succeeds on loopback; an unauthenticated delivery request is 401.
2. The public endpoint has a trusted, hostname/IP-matching certificate; HTTP cannot
   access the authenticated API. Never use `curl -k` for this check.
3. A synthetic non-image file uploads in tus chunks, stops and resumes at the
   persisted offset, and arrives at Linux with identical bytes and SHA-256.
4. ACK removes the final pending content reference; retries create no duplicate.
5. A MiRelay restart preserves interrupted upload state and completed metadata.
6. Existing proxy and SSH processes/listeners remain unchanged, and 8080 is not
   listening on a public interface.

Do not point an acceptance-test receiver at a non-empty user queue: normal sync
acknowledges and removes pending server content. Use a separate test instance/token
or verify that a new instance is empty and contains only synthetic test deliveries.

Keep credentials and test reports outside Git. Do not put tokens in command-line
arguments, screenshots, public logs, or shared troubleshooting output.

## Repeatable smoke test

Build the debug CLI binaries first with `cargo build --locked --bins`. Privately
set `MIRELAY_TOKEN` in the environment, then run:

```bash
bash deploy/smoke-test.sh https://relay.example.com
```

The script requires `curl`, `jq`, `openssl`, and an **otherwise unused queue**.
It checks missing/wrong tokens, required API version negotiation, persisted tus
offsets, binary/text delivery, exact bytes, ACK and an empty second sync. It rejects
a non-empty initial queue and unexpected files before receiving, but these checks
are not a lock against concurrent senders: do not use it on an active user queue.

An optional second argument is an SSH alias. Supplying it explicitly authorizes
the test to restart only `mirelay-server.service` between the first chunk and
resumption. For a pre-established SSH tunnel, the script also accepts
`http://127.0.0.1:PORT`; this tests the backend, **not public HTTPS or the phone**.
Private reports stay under `target/deployment/smoke.*`; the temporary authorization
header file is removed on exit. No test token or actual server domain is committed.

References: [Caddy automatic HTTPS](https://caddyserver.com/docs/automatic-https),
[HTTP protocol selection](https://caddyserver.com/docs/caddyfile/options#protocols),
[reverse proxy](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy),
[Let's Encrypt IP certificates](https://letsencrypt.org/2026/01/15/6day-and-ip-general-availability.html),
[ACME issuer profile](https://caddyserver.com/docs/caddyfile/directives/tls#acme).
