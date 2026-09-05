#!/usr/bin/env bash
set -euo pipefail

preview_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
workspace_name=default
registry_name=folders.toml
prepare_only=false
desktop_args=()

while (($#)); do
  case "$1" in
    --workspace)
      if (($# < 2)) || [[ ! "$2" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]*$ ]]; then
        echo 'Expected --workspace NAME (letters, digits, underscores or hyphens).' >&2
        exit 2
      fi
      workspace_name="$2"
      shift 2
      ;;
    --empty) registry_name=empty.toml; shift ;;
    --prepare) prepare_only=true; shift ;;
    --help)
      echo 'Usage: dev/frontend/run.sh [--workspace NAME] [--empty] [--prepare] [desktop options]'
      echo 'See dev/frontend/README.md for screenshots and fixture limitations.'
      exit 0
      ;;
    *) desktop_args+=("$1"); shift ;;
  esac
done

preview_root="$preview_dir/.local/$workspace_name"
cd -- "$preview_dir/../.."
cargo run --quiet --locked --features desktop --example frontend-fixture -- "$preview_root"
if "$prepare_only"; then
  exit 0
fi
exec cargo run --quiet --locked --features desktop --bin mirelay-desktop -- \
  --new-instance --registry "$preview_root/$registry_name" "${desktop_args[@]}"
