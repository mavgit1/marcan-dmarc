#!/usr/bin/env bash
# The GitHub release entry of a version tag: its title and text come from the tag's
# message (which publish.sh also uses as the public commit message), followed by where
# the full notes and the image are.
#
#   scripts/github-release.sh --print vX.Y.Z   print the title and the text, change nothing
#   scripts/github-release.sh vX.Y.Z           create the entry with gh, unless it exists
#
# The second form runs in GitHub Actions (.github/workflows): it needs gh, GH_TOKEN and
# GH_REPO. Only the highest version is marked as the latest release.
set -euo pipefail

DOCS=https://marcan.kaitiaki.tech
IMAGE=ghcr.io/mavgit1/marcan

print=false
if [[ "${1:-}" == --print ]]; then print=true; shift; fi
tag=${1:?usage: scripts/github-release.sh [--print] vX.Y.Z}
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "tag '$tag' is not vX.Y.Z" >&2; exit 1; }
version=${tag#v}

# actions/checkout leaves a pushed tag pointing at the commit, without its message.
if ! $print; then
	git fetch --quiet --force origin "refs/tags/$tag:refs/tags/$tag"
fi
[[ "$(git cat-file -t "$tag" 2>/dev/null)" == tag ]] || { echo "$tag is not an annotated tag here" >&2; exit 1; }

title=$(git tag -l --format='%(contents:subject)' "$tag")
[[ -n "$title" ]] || title="Marcan $version"

# The message is wrapped at 72 columns; GitHub keeps every line break of a release text,
# so each paragraph becomes one line.
body=$(git tag -l --format='%(contents:body)' "$tag" | awk '
	/^[[:space:]]*$/ { if (line != "") { print line; print ""; line = "" } next }
	{ line = (line == "" ? $0 : line " " $0) }
	END { if (line != "") print line }')

notes=$(printf '%s\n\n%s\n\n%s\n' \
	"$body" \
	"Full release notes: $DOCS/releases#_${version//./-}" \
	"Image: \`$IMAGE:$version\`. Upgrading: $DOCS/upgrade")

if $print; then
	printf '%s\n\n%s\n' "$title" "$notes"
	exit 0
fi

if gh release view "$tag" >/dev/null 2>&1; then
	echo "$tag already has a release entry"
	exit 0
fi

highest=$(git ls-remote --tags --refs origin 'v*.*.*' | sed 's|.*refs/tags/||' | sort -V | tail -n1)
latest=false
[[ "$tag" == "$highest" ]] && latest=true

gh release create "$tag" --verify-tag --title "$title" --notes "$notes" --latest="$latest"
