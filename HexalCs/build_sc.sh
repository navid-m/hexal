#!/bin/sh
# Backwards-compatible entry point for the obfuscated single-file build.
# With no arguments it behaves like "./build.sh -s"; explicit flags are
# forwarded unchanged (so "./build_sc.sh -a" also works).

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

if [ "$#" -eq 0 ]; then
    set -- -s
fi

exec "$SCRIPT_DIR/build.sh" "$@"
