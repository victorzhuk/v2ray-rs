#!/usr/bin/env bash
# Bakes the release version and artifact digest into a copy of install.sh.
# The repository copy keeps its @VERSION@/@SHA256@ placeholders and falls back
# to the latest release with no digest check.
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "usage: gen-installer.sh VERSION ARTIFACT_PATH OUTPUT_PATH" >&2
    exit 2
fi

version=$1
artifact=$2
output=$3
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

[[ -f $artifact ]] || { echo "no such artifact: $artifact" >&2; exit 1; }
sha=$(sha256sum -b "$artifact" | awk '{print $1}')

sed -e "s|@VERSION@|${version}|" -e "s|@SHA256@|${sha}|" \
    "$repo/scripts/install.sh" >"$output"
chmod +x "$output"

# Assert the *result*, not merely that a substitution happened. The generated
# installer decides whether it is pinned by inspecting the shape of these two
# values, so that is exactly what has to hold here.
assert_pinned() {
    local name=$1 pattern=$2 value
    value=$(sed -n "s/^${name}=\"\(.*\)\"\$/\1/p" "$output")
    if [[ ! $value =~ $pattern ]]; then
        echo "generated installer has an unusable ${name}: '${value}'" >&2
        exit 1
    fi
}

assert_pinned APP_VERSION '^[0-9]'
assert_pinned ARTIFACT_SHA256 '^[0-9a-f]{64}$'

if grep -q '@VERSION@\|@SHA256@' "$output"; then
    echo "placeholders left unsubstituted in $output" >&2
    exit 1
fi

echo "$output ($version, sha256 $sha)"
