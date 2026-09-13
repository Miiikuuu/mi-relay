# Folder setup and pairing

This changes setup, authorization and source initialization, **not** bidirectional
sync. Delivery remains Android → server → Linux. There is no Linux auto-send,
Android receiver or deletion propagation. Opt-in [directory sync](directory-sync.md)
adds original relative paths and later edits on top of this pairing flow;
the delivery-only workflow below remains available.

## User flow

1. Configure a separate `MIRELAY_ADMIN_TOKEN` on the backend (at least 32 bytes;
   prefer 32 random bytes encoded as hex). The existing legacy token/queue stays.
2. Linux **Add Folder**: choose **Folder Path**, the original HTTPS server URL and
   a name. In **Pairing → Administrator credential**, paste only the administrator
   token value and click **Create on server**. **Session Token** is not the
   administrator field; creation fills it with a scoped receiver token, alongside
   the scoped URL and a ten-minute invitation. Leading/trailing whitespace is
   trimmed, but internal whitespace and non-ASCII characters are rejected.
   Creation feedback appears beside the administrator field. Failure preserves
   its input for correction; only successful creation clears it. The administrator
   token is not saved to configuration files.
3. Linux **Show QR code**, then Android **Add Folder → Scan QR code** fills the
   HTTPS server URL and invitation. Review the address; scanning is not consent
   to connect. Manual entry remains available. **Choose existing directory** selects the directory
   already used by Pixiv or another app. The system picker must grant read access;
   MiRelay cannot bypass private app storage/provider restrictions. Enter the
   original server URL and temporary pairing code, never the admin/receiver token.
4. Android displays **Awaiting pairing** and a verification code. Linux **Check
   pairing** displays the same code; compare both, then **Codes match — confirm**.
   Android **Check pairing** refreshes its state. The server refuses transfers
   until confirmed. Saved pending Folders remain blocked; their live state can be
   checked in Folder Settings. Save runs connection checks off the GTK thread.
5. The Android source is initialized in place, with read permission and a complete
   metadata baseline. **Also send existing files** defaults off. Originals are not
   moved, overwritten or deleted. Auto stays off until explicitly enabled.
6. First activation preserves that baseline and the history choice, including
   files arriving while pairing was pending. Further pause/re-enable uses the
   existing fresh-baseline behavior. Stable metadata and size limits still apply.

Legacy connections remain available by unchecking **Pair with Linux** on Android.
Creation/settings now verify the connection before saving; the legacy check reads
at most one delivery descriptor, never downloads or acknowledges it.

**Persistence limitation:** Linux still keeps entered/generated receiver tokens
only for the process session. Keep a private copy from Folder Settings, or provide
the configured environment variable on restart. Keyring storage is not implemented.
Android credentials are encrypted by Android Keystore. [QR invitations](qr-pairing.md)
are short-lived secrets and must not be shared publicly; neither QR generation nor
scanning changes the server's two-party confirmation requirement.

## Setup protocol

HTTPS, `Mirelay-Protocol-Version: 1`, `Cache-Control: no-store`. Credentials/codes
must never be logged. Client responses are bounded to 16 KiB, redirects rejected,
and setup mutations are not automatically retried.

| Endpoint | Authority | Request / purpose |
| --- | --- | --- |
| `POST /api/v1/folders` | Administrator Bearer | `{ "name": "Pixiv" }`; returns Folder ID, receiver token, pairing code and expiry |
| `POST /api/v1/pairings/claim` | Invitation in body | `{ "pairing_code": "<id>.<secret>", "sender_token": "<64 hex>" }`; claims one sender |
| `GET /f/<id>/api/v1/handshake` | This Folder's sender/receiver | Identity, role, state, verification code, max file size |
| `POST /f/<id>/api/v1/pairing/confirm` | Receiver | `{ "verification": "<12 hex>" }`; confirms the exact current sender |
| `POST /f/<id>/api/v1/pairing/renew` | Receiver | Revokes the old sender/invitation and returns a new invitation |

States: `awaiting_peer`, `awaiting_confirmation`, `ready`. Invalid credentials or
invitations: 401; wrong role: 403; pending/stale confirmation: 409; incompatible
protocol: 426. An absent admin credential never promotes the legacy credential.

The existing clients use scoped base URL `https://host/f/<id>` and append their
normal `api/v1/uploads` / `api/v1/deliveries` paths. Relative tus Location preserves
the Folder prefix during resume. The token must match the chosen Folder **and**
role on every request. Senders cannot list/download/ACK; receivers cannot upload;
neither can create another Folder. Legacy tokens cannot access scoped endpoints.

Server credentials and invitations are stored only as SHA-256 hashes. Claiming is
transactional: identical invitation+sender retries succeed, a different sender is
rejected. Android persists its encrypted draft credential before claiming, allowing
recovery after process death or a lost response. Renewal invalidates old claims and
stale confirmations; already-authorized in-flight requests may finish.

Queues use reserved `folder_<uuid>` identities and reuse transactional object
storage. Objects survive until all pending references are acknowledged. Server
schema 1 migrates additively to 2; Android schema 1/2 migrates to 3, retaining old
deliveries/credentials. Back up first; do not run an old server against schema 2.

Each Folder currently has one sender and one receiver. Lost create responses may
leave empty, unreachable pending Folders; creation is not blindly retried. There
is a 1,000-Folder safety ceiling, but not a full admin deletion/recovery/quota UI.
Use [deployment scripts](../deploy/README.md) only after reviewing their plan.
