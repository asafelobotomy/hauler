#!/usr/bin/env bash
# Fail when a source file is longer than the limit (400 lines by default),
# listing each one. Every line counts: code, comments, blanks and tests.
#
#   scripts/check-loc.sh [limit]
set -euo pipefail

LIMIT=${1:-400}
cd "$(dirname "$0")/.."

over=0
while IFS= read -r -d '' file; do
    lines=$(wc -l <"$file")
    if ((lines > LIMIT)); then
        echo "$file: $lines lines (limit $LIMIT)"
        over=1
    fi
done < <(find crates scripts -path '*/target' -prune -o -type f \
    \( -name '*.rs' -o -name '*.qml' -o -name '*.cpp' -o -name '*.h' -o -name '*.sh' \) -print0 | sort -z)

if ((over)); then
    echo "Split the files above into smaller modules." >&2
    exit 1
fi
