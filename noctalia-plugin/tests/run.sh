#!/bin/sh
# Runs each plugin test as one chunk: the stubs, the entry script, the test.
# Needs `luau` (nix shell nixpkgs#luau -c sh noctalia-plugin/tests/run.sh).
set -eu
dir=$(dirname "$0")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for pair in panel:panel_test bar:bar_test shortcut:shortcut_test; do
  entry=${pair%%:*}
  test=${pair##*:}
  cat "$dir/stubs.luau" "$dir/../$entry.luau" "$dir/$test.luau" > "$tmp/$test.luau"
  luau "$tmp/$test.luau"
done
