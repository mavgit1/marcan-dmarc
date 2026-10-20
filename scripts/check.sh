#!/usr/bin/env bash
# Everything a change must pass: Rust formatting, tests and lints, the UI type check, lint,
# formatting and tests, TypeScript bindings that match the Rust API, and a docs build without dead links.
#
#   scripts/check.sh
#
# Installs the UI and docs dependencies from their lockfiles when node_modules is
# missing, and builds the UI first: the server embeds web/build and its tests serve it.
#
# `cargo test` regenerates web/src/lib/types (ts-rs); the directory is wiped
# first so a removed Rust type also removes its binding. Commit the result.
set -euo pipefail
cd "$(dirname "$0")/.."

for dir in web docs; do
	[[ -d "$dir/node_modules" ]] || pnpm --dir "$dir" install --frozen-lockfile
done
pnpm --dir web build >/dev/null

rm -rf web/src/lib/types
cargo fmt --all --check

# The dependency tree: no known advisory (cargo-audit, unmaintained and yanked
# crates included), and only the licenses, crates and sources deny.toml allows
# (cargo-deny). Both are installed on first use. An advisory that cannot be
# fixed is ignored in deny.toml's [advisories] list with its reason; cargo-audit
# reads the ids from there, so the list is kept in one place.
for tool in cargo-audit cargo-deny; do
	command -v "$tool" >/dev/null || cargo install --locked "$tool"
done
audit_ignores=()
while read -r id; do
	audit_ignores+=(--ignore "$id")
done < <(sed -n '/^\[advisories\]/,/^\[/s/.*id = "\(RUSTSEC-[0-9]*-[0-9]*\)".*/\1/p' deny.toml)
cargo audit --quiet --deny warnings ${audit_ignores[@]+"${audit_ignores[@]}"}
cargo deny --log-level error check licenses bans sources
cargo test --workspace --quiet
cargo clippy --workspace --all-targets --quiet -- -D warnings
(cd web && pnpm check && pnpm lint && pnpm format:check && pnpm test)
(cd docs && pnpm build >/dev/null)

# docs/licenses/bcl-1.0.md is the frozen text of BCL 1.0 (contracts link to it). While
# LICENSE is that version, both must match; a new version gets a new page instead.
if grep -q '^ *Version 1\.0$' LICENSE && ! diff -q LICENSE <(awk '/^```text$/{f=1;next} /^```$/{f=0} f' docs/licenses/bcl-1.0.md) >/dev/null; then
	echo "LICENSE differs from docs/licenses/bcl-1.0.md: a changed license needs a new version and page." >&2
	exit 1
fi

if ! git diff --quiet -- web/src/lib/types || [[ -n "$(git ls-files --others --exclude-standard web/src/lib/types)" ]]; then
	echo "web/src/lib/types changed: the Rust API types moved. Review and commit the bindings." >&2
	git status --short -- web/src/lib/types >&2
	exit 1
fi
