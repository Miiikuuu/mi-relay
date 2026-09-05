#!/usr/bin/env bash
# Mutates ONLY an explicitly selected, otherwise unused test queue. Keeps reports.
set +x
set -euo pipefail
umask 077

endpoint="${1:-}"
restart_host="${2:-}"
insecure=()
if [[ "$endpoint" =~ ^http://127\.0\.0\.1:[0-9]+$ ]]; then
    # For an already established SSH tunnel, never a public cleartext listener.
    insecure=(--allow-insecure-http)
elif [[ ! "$endpoint" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ ]]; then
    echo 'Usage: MIRELAY_TOKEN=... bash deploy/smoke-test.sh https://host [SSH-alias-to-restart-MiRelay]' >&2
    exit 2
fi
if [[ -n "$restart_host" && ! "$restart_host" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
    echo 'Invalid SSH alias.' >&2
    exit 2
fi
if [[ ! "${MIRELAY_TOKEN:-}" =~ ^[A-Za-z0-9._~-]+$ ]]; then
    echo 'Set MIRELAY_TOKEN privately in the environment; no token is printed.' >&2
    exit 2
fi
repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
uploader="$repo_dir/target/debug/mirelay-upload"
receiver="$repo_dir/target/debug/mirelay"
test -x "$uploader"
test -x "$receiver"
mkdir -p "$repo_dir/target/deployment"
report="$(mktemp -d "$repo_dir/target/deployment/smoke.XXXXXXXX")"
trap 'rm -f -- "$report/auth-header"' EXIT
printf 'Authorization: Bearer %s\n' "$MIRELAY_TOKEN" > "$report/auth-header"
# These probes are GET/HEAD only. Retry transient failures with a bounded budget;
# never bypass TLS checks or retry arbitrary HTTP errors (such as 401/426).
request=(curl --noproxy '*' --silent --show-error --connect-timeout 5 --max-time 15 --retry 2 --retry-max-time 25 --retry-connrefused)
auth_request=("${request[@]}" --fail --header "@$report/auth-header" --header 'Mirelay-Protocol-Version: 1')
index_url="$endpoint/api/v1/deliveries?status=pending&limit=50"

echo "REPORT=$report"
test "$("${request[@]}" --fail "$endpoint/healthz")" = ok
test "$("${request[@]}" --output /dev/null --write-out '%{http_code}' "$index_url")" = 401
test "$("${request[@]}" --header 'Authorization: Bearer deliberately-invalid' --output /dev/null --write-out '%{http_code}' "$index_url")" = 401
test "$("${request[@]}" --header "@$report/auth-header" --output /dev/null --write-out '%{http_code}' "$index_url")" = 426
"${auth_request[@]}" "$index_url" | jq -e '.items == [] and .next_cursor == null' > /dev/null

openssl rand -out "$report/payload.bin" 2097152
printf 'MiRelay synthetic UTF-8 delivery test.\n非图片文件验证。\n' > "$report/notes.txt"
binary_hash="$(sha256sum "$report/payload.bin" | cut -d ' ' -f 1)"
text_hash="$(sha256sum "$report/notes.txt" | cut -d ' ' -f 1)"
upload_args=("$report/payload.bin" --server-url "$endpoint" --state-file "$report/upload.json" --chunk-size-bytes 524288 --request-timeout-seconds 45 "${insecure[@]}")
"$uploader" "${upload_args[@]}" --max-chunks 1
upload_url="$(jq -er '.upload_url' "$report/upload.json")"
[[ "$upload_url" == "$endpoint/api/v1/uploads/"* ]]

check_offset() {
    "${auth_request[@]}" --head --header 'Tus-Resumable: 1.0.0' "$upload_url" > "$report/$1"
    test "$(awk -F ': *' 'tolower($1) == "upload-offset" {gsub("\r", "", $2); print $2}' "$report/$1")" = 524288
}
check_offset before-resume.headers
if [[ -n "$restart_host" ]]; then
    # Explicit opt-in: never restart SSH, the VPN, or another reverse proxy.
    ssh -o BatchMode=yes -o ConnectTimeout=8 -o StrictHostKeyChecking=yes "$restart_host" \
        'systemctl restart mirelay-server.service && curl --noproxy "*" --fail --silent --show-error --retry 3 --retry-connrefused --max-time 5 http://127.0.0.1:8080/healthz'
    check_offset after-restart.headers
fi
"$uploader" "${upload_args[@]}"
test ! -e "$report/upload.json"
"$uploader" "$report/notes.txt" --server-url "$endpoint" --state-file "$report/text-upload.json" --request-timeout-seconds 45 "${insecure[@]}"
"${auth_request[@]}" "$index_url" > "$report/pending.json"
# Refuse to run a receiver if another sender has added user files in the meantime.
jq -e --arg binary "$binary_hash" --arg text "$text_hash" \
    '(.items | length) == 2 and .next_cursor == null and
     ([.items[].sha256] | sort) == ([$binary, $text] | sort) and
     ([.items[].original_name] | sort) == ["notes.txt", "payload.bin"]' \
    "$report/pending.json" > /dev/null
"$receiver" --config "$report/client.toml" init --data-dir "$report/client" --server-url "$endpoint" "${insecure[@]}"
"$receiver" --config "$report/client.toml" sync
jq -e '(.deliveries | length) == 2 and all(.deliveries[];
    .delivery_status == "acknowledged" and .wallpaper_status == "not_applicable")' \
    "$report/client/state.json" > /dev/null
for name in payload.bin notes.txt; do
    stored="$(jq -er --arg name "$name" '.deliveries[] | select(.original_name == $name) | .stored_path' "$report/client/state.json")"
    [[ "$stored" == "$report/client/"* ]]
    cmp -- "$report/$name" "$stored"
done
"$receiver" --config "$report/client.toml" sync
"${auth_request[@]}" "$index_url" | jq -e '.items == [] and .next_cursor == null' > /dev/null
echo 'PASS: health, missing/wrong auth, tus offset/resume, two non-image files, byte equality, ACK and empty re-sync.'
