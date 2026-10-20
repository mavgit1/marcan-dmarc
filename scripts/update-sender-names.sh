#!/usr/bin/env bash
# Refresh the vendored parsedmarc sender name data in crates/dmarc-identity/data.
#
#   scripts/update-sender-names.sh [ref]    # default: master
#
# Downloads both files at one upstream commit, prints what changed (added,
# removed and renamed operator domains) and leaves the result for review with
# `git diff`. Nothing is fetched at runtime: Marcan embeds these files.
set -euo pipefail

REPO=domainaware/parsedmarc
MAPS=parsedmarc/resources/maps
DATA="$(cd "$(dirname "$0")/.." && pwd)/crates/dmarc-identity/data"
REF="${1:-master}"

SHA=$(git ls-remote "https://github.com/$REPO.git" "$REF" | head -n1 | cut -f1)
if [[ -z "$SHA" ]]; then
	SHA="$REF" # already a commit
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

for f in base_reverse_dns_map.csv psl_overrides.txt; do
	curl -fsSL "https://raw.githubusercontent.com/$REPO/$SHA/$MAPS/$f" -o "$TMP/$f"
done

# domain<TAB>name, sorted, for a readable comparison (names may be quoted).
names() {
	python3 - "$1" <<'PY'
import csv, sys
with open(sys.argv[1], newline='', encoding='utf-8') as f:
    rows = csv.reader(f)
    next(rows, None)
    for row in sorted(r for r in rows if len(r) >= 2):
        print(f"{row[0].strip().lower()}\t{row[1].strip()}")
PY
}

names "$DATA/base_reverse_dns_map.csv" >"$TMP/old.tsv"
names "$TMP/base_reverse_dns_map.csv" >"$TMP/new.tsv"

cut -f1 "$TMP/old.tsv" >"$TMP/old.keys"
cut -f1 "$TMP/new.tsv" >"$TMP/new.keys"
added=$(comm -13 "$TMP/old.keys" "$TMP/new.keys" | wc -l | tr -d ' ')
removed=$(comm -23 "$TMP/old.keys" "$TMP/new.keys" | wc -l | tr -d ' ')
renamed=$(join -t $'\t' "$TMP/old.tsv" "$TMP/new.tsv" | awk -F'\t' '$2 != $3' | wc -l | tr -d ' ')

echo "parsedmarc $(cat "$DATA/SOURCE") -> $SHA"
echo "  map: +$added domains, -$removed domains, $renamed renamed"
comm -23 "$TMP/old.keys" "$TMP/new.keys" | sed 's/^/  removed: /' | head -n 20
join -t $'\t' "$TMP/old.tsv" "$TMP/new.tsv" | awk -F'\t' '$2 != $3 {print "  renamed: " $1 ": " $2 " -> " $3}' | head -n 20
diff -u "$DATA/psl_overrides.txt" "$TMP/psl_overrides.txt" | sed -n '3,$p' | grep '^[-+]' | sed 's/^/  psl_overrides /' || true

cp "$TMP/base_reverse_dns_map.csv" "$TMP/psl_overrides.txt" "$DATA/"
echo "$SHA" >"$DATA/SOURCE"
echo "Updated $DATA. Review with git diff, then run: cargo test -p dmarc-identity"
