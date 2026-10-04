#!/usr/bin/env bash
# A CAD-source review job. Failed checks retain the blocked bundle and reports.
# This script performs no checkout, commit, push, or comment publication.
set -euo pipefail
PROJECT_PATH="${CAD_REVIEW_PROJECT:?Set CAD_REVIEW_PROJECT to an existing CAD project directory}"
BASE_REVISION="${CAD_REVIEW_BASE:-HEAD~1}"
HEAD_REVISION="${CAD_REVIEW_HEAD:-HEAD}"
OUTPUT_DIRECTORY="${CAD_REVIEW_OUT:-build/cad-review-ci}"
cargo run -p cad-cli -- review-bundle "$PROJECT_PATH" --base "$BASE_REVISION" --head "$HEAD_REVISION" --out "$OUTPUT_DIRECTORY"
cargo run -p cad-cli -- verify-review-bundle "$OUTPUT_DIRECTORY"
