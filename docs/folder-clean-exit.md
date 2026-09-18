# Clean Folder exit

Clean exit is distinct from **Disconnect** (revoke transfer access but keep
data) and **Remove locally** (legacy local-only removal). It requires independent
paired Folder credentials and an updated relay. It never deletes the phone's
source documents or Linux's received/original files, including conflict copies.
Unsent private staging copies and pending relay deliveries are discarded only
after the user confirms **Clean exit**.

## Recoverable stages

1. The initiating client persists a stopped state before contacting the relay.
2. The relay permanently revokes both transfer credentials, invalidates pairing,
   and records an exit tombstone in one SQLite transaction.
3. The relay drains upload/content writers, removes owned tus records/parts,
   removes content with no pending references from another Folder, then deletes
   delivery and directory metadata. Ownership records remain until filesystem
   deletion succeeds, so interrupted cleanup is retryable.
4. Each client stops and drains its local file work, removes its app-owned data,
   and acknowledges its own cleanup. Credentials cannot acknowledge the other
   role. An offline or old client remains pending, not silently counted as done.
5. Only both client receipts plus relay cleanup produce **Clean exit complete**.
   The visible local receipt can then be removed.

An already-authorized request may finish while work is being drained. Exit is
not an instantaneous remote kill switch; completion waits for local cleanup,
and any file already received on Linux remains an original-file preservation
target rather than something to delete retroactively.

The relay intentionally retains a small tombstone and credential hashes for
authenticated status/retry requests. These credentials cannot transfer, renew
pairing or reconnect. Local encrypted Android receipt credentials remain until
completion, then are cleared. Linux continues using its existing in-memory or
environment-supplied credential; after restarting, a missing credential must
be entered in Folder settings. No new secret file or background service is added.

## Resuming

- Android retries an already requested exit on app startup. It also checks
  connected paired Folders for peer-requested exits on startup, off the UI
  thread; **Retry / check clean exit** is available in Folder settings.
- Linux checks for peer exits before its receive/check cycle. Known pending
  exits participate in the existing automatic cycle when their credential is
  available. Otherwise open Folder settings and supply the receiver token to
  retry. Auto receive being disabled does not imply an always-running peer
  notification channel.
- The relay retries unfinished cleanup at service startup or `reconcile`.
  Individual cleanup failures stay revoked and pending without preventing
  unrelated Folders from being served. Repeating the exit request also retries.
- A missing/old server API is not success and does not fall back to destructive
  local-only removal. Upgrade the relay first. Older apps do not acknowledge
  cleanup automatically; update both clients to complete the workflow.

## Local cleanup boundaries

Android deletes transfer-owned private staging/resume files, derived transfer
previews, transfer/Auto/directory records and unused persisted source-directory
read grants. A grant still referenced by another Folder is kept. Actual work
is drained with a bounded lock wait; a blocked provider leaves cleanup pending.
New staging persists a Folder ownership marker before writing payload bytes,
so cleanup can also find an interrupted import that never reached the queue.
Ownership is removed last. Old staging payloads without either a transfer record
or a valid ownership marker require review; cleanup never guesses their owner.
Legacy removal may leave an empty, regular `resume.json.lock` file. Such a lock
is retained and does not block another Folder's cleanup. Nonempty locks,
symbolic links and unowned payload/resume data still require review.

Linux only automatically cleans state whose paths match a desktop-created
Folder's owned config/data layout. Imported/custom layouts, symlinks, unexpected
file types or overlap with a user library require review; arbitrary configured
paths are never recursively deleted. Retirement markers and lock inodes remain
to prevent stale CLI/worker instances from restarting or bypassing locks. The
config acts as the pending receipt; removing a completed receipt deletes that
owned config file. Original file trees are not cleanup targets.

Legacy shared-token Folders such as older pixiv configurations do not have
independent revocation authority. They retain **Remove locally**; this action
must not rotate a shared server token or delete data assigned to another client.
Unattributable old tus fragments block a claimed complete relay purge and need
administrator review instead of guessed ownership.

This is logical cleanup, not forensic secure erasure of SSDs, database free
pages, filesystem snapshots, operating-system caches or external backups.

## Protocol and compatibility

- `GET /f/{id}/api/v1/pairing/exit`: authenticated status, without starting exit.
- `POST /f/{id}/api/v1/pairing/exit`: permanently request/retry relay cleanup.
- `POST /f/{id}/api/v1/pairing/exit/ack`: acknowledge the authenticated role's
  local cleanup after relay cleanup succeeds.
- A valid acknowledgement before exit is requested or while relay cleanup is
  pending returns HTTP 409 `exit_cleanup_pending`, without advancing receipts.
- Receipt fields: `schema_version`, `folder_id`, `role`, `requested`,
  `server_cleaned`, `sender_cleaned`, `receiver_cleaned`.
- Server database schema 5 adds `folder_exits`; Android schema 7 adds
  `exit_phase`. Older database readers must not be used after upgrading.

Development validation and live deployment are separate. Do not invoke clean
exit on real Folders as a smoke test; use a disposable paired fixture.

## Development validation

See the [2026-09-15 evidence index](validation/folder-clean-exit-2026-09-15.json)
for exact APK hashes, commands/reports, passing scopes, failed attempts and
deployment limits. Rust checks, Android unit tests, isolated end-to-end exit
cases, normal transfer regressions and manual/Auto process-death recovery were
exercised. The final receipt-race SQL guard received an additional targeted APK
run; this is not a claim that every test was repeated on every intermediate APK.
