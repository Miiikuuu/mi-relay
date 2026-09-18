# Security policy

MiRelay is experimental software. No stable release or long-term security
support window is currently offered. Fixes target the current development branch;
historical validation reports do not establish that a version is secure.

## Reporting a vulnerability

Do not post credentials, private keys, pairing QR codes, private media, live
server addresses or an exploitable proof of concept in a public issue.

If this repository's **Security → Report a vulnerability** option is available,
use that private channel. If it is unavailable, open an issue containing only a
request for a private security contact; wait for a private channel before
sharing sensitive details. This document does not imply that private reporting
has already been enabled on GitHub.

Include the affected commit/version, component, impact, and reproduction steps
using a local test relay and synthetic files. Redact tokens, personal paths and
device identifiers from logs. Do not test against another person's relay.

## Important boundaries

- HTTPS is required outside explicitly isolated local tests. Do not disable
  certificate validation or expose the HTTP backend directly to the Internet.
- There is **no end-to-end encryption**. The relay operator can read contents
  and metadata. Use a relay you trust and protect its storage and credentials.
- Administrator, sender, receiver and invitation credentials are different.
  Keep administrator credentials off the sending device. Pairing screenshots
  are sensitive while their invitations remain valid.
- Directory sync is Android → Linux, not a two-way mirror or a backup system.
  Retain independent backups. Limits, retained history and unfinished uploads
  require disk-capacity planning.
- Signing keys, keystores, passwords, runtime databases and private device-test
  artifacts must never be committed. Android distribution signing is local;
  do not place the signing private key in CI.

See [release validation status](RELEASE_VALIDATION.md) for known acceptance gaps.
