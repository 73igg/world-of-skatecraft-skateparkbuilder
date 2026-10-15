#!/usr/bin/env bash
# Upgrade an existing deployment, preserving its database. Normal setup runs this automatically.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
exec python3 "$ROOT/setup/migrate_database.py" upgrade --server "${1:-$ROOT/server}"
