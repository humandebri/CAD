#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

pnpm --dir apps/viewer install --frozen-lockfile
pnpm --dir apps/viewer exec playwright install chromium
