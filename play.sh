#!/usr/bin/env bash
# Starts the local server (if it is not running) and the game. Run setup/setup.sh first.
# In game: J summons/dismisses the skateboard (Xbox-style controller), K swaps the skate camera.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cd "$ROOT"
if [ -e /etc/NIXOS ] && [ -z "${IN_NIX_SHELL:-}" ]; then
    exec nix-shell "$ROOT/shell.nix" --run "$(printf '%q ' "$0" "$@")"
fi
[ -x target/release/benilla ] || { echo "Not built yet: run setup/setup.sh first."; exit 1; }
SERVER=$(python3 "$ROOT/setup/migrate_database.py" path)
python3 "$ROOT/setup/migrate_database.py" check --server "$SERVER"
docker compose -p "${SKATECRAFT_SERVER_PROJECT:-world-of-skatecraft}" --project-directory "$SERVER" -f "$SERVER/compose.yaml" up -d --wait --wait-timeout 180
python3 "$ROOT/setup/migrate_database.py" check --live --server "$SERVER"
export WOW_PARK_ASSETS="$ROOT/benilla-config/park-assets"
CATALOG_PYTHON="$ROOT/.setup/venv/bin/python"
if [ ! -e "$WOW_PARK_ASSETS/skate-catalog.json" ]; then
    [ -x "$CATALOG_PYTHON" ] || { echo "The object catalog converter is missing: run setup first."; exit 1; }
    "$CATALOG_PYTHON" tools/prepare_park_catalog.py --out "$WOW_PARK_ASSETS"
fi
if [ ! -e "$WOW_PARK_ASSETS/wow-names.json" ] && [ -x "$CATALOG_PYTHON" ]; then
    "$CATALOG_PYTHON" tools/cache_park_object_names.py --container "${SKATECRAFT_SERVER_PROJECT:-world-of-skatecraft}-database-1" --out "$WOW_PARK_ASSETS/wow-names.json"
fi
export WOW_HOST=localhost
export WOW_SKATE_ASSETS="$ROOT/skate-data/assets"
export WOW_SKATE_AUDIO="$ROOT/skate-audio"
exec target/release/benilla "$@"
