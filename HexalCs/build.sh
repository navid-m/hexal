#!/bin/sh
# Build Hexal.
#
# The default target is the NativeAOT binary used by install.sh. Use -s or -a
# for portable, single-file releases, which are obfuscated with Obfuscar.

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SCRIPT_DIR"

PROJECT="Hexal/Hexal.csproj"
CONFIGURATION="Release"
OUTPUT_ROOT="dist"
MODE="aot"
RID=""
OBFUSCATE=1
PUBLISH=0
RELEASE_REPO="${HEXAL_RELEASE_REPO:-navid-m/hexal-dist}"

usage() {
    cat <<'EOF'
Usage: ./build.sh [-s | -a | -p] [-r RID] [-o DIR] [-n] [-h]

  (no options)  NativeAOT publish for the current platform. The native
                binary contains no managed IL, so it is not obfuscated.
  -s            Self-contained single-file publish for the current platform,
                obfuscated with Obfuscar.
  -a            Publish all supported platforms as self-contained single-file
                builds (not AOT), obfuscated, with one folder per platform:
                  osx-arm64, linux-x64, win-x64
                and pack them into a single zip archive.
  -p            Like -a, then upload the zip as the latest release of
                navid-m/hexal-dist with the gh CLI. The download URL is
                always:
                  https://github.com/navid-m/hexal-dist/releases/latest/download/hexal-all-platforms.zip
  -r RID        Runtime identifier for -s / the default build
                (default: detected from the host).
  -o DIR        Output directory for -s / -a / -p artifacts (default: dist).
  -n            Skip obfuscation (single-file modes only; for local testing).
  -h            Show this help.

Examples:
  ./build.sh               # fast NativeAOT binary for this machine
  ./build.sh -s            # obfuscated single-file build for this machine
  ./build.sh -a            # obfuscated builds for macOS arm64, Linux x64
                           # and Windows x64, all in one zip
  ./build.sh -p            # same as -a, then publish the zip to GitHub
EOF
}

die() {
    echo "error: $*" >&2
    exit 1
}

host_rid() {
    os=$(uname -s)
    arch=$(uname -m)

    case "$os" in
        Linux) os=linux ;;
        Darwin) os=osx ;;
        *) die "unsupported operating system '$os'" ;;
    esac

    case "$arch" in
        x86_64|amd64) arch=x64 ;;
        arm64|aarch64) arch=arm64 ;;
        *) die "unsupported architecture '$arch'" ;;
    esac

    printf '%s-%s' "$os" "$arch"
}

publish_single_file() {
    rid=$1
    destination=$2
    obfuscate_arg=false

    if [ "$OBFUSCATE" -eq 1 ]; then
        obfuscate_arg=true
    fi

    rm -rf "$destination"
    mkdir -p "$destination"

    dotnet publish "$PROJECT" \
        -c "$CONFIGURATION" \
        -r "$rid" \
        --self-contained true \
        -p:PublishAot=false \
        -p:PublishSingleFile=true \
        -p:IncludeNativeLibrariesForSelfExtract=true \
        -p:Obfuscate="$obfuscate_arg" \
        -p:DebugType=None \
        -p:DebugSymbols=false \
        -o "$destination"

    # Single-file already keeps symbols out of the bundle; drop any that were
    # copied next to the executable as well so release artifacts do not leak
    # the original (unobfuscated) names and source paths.
    find "$destination" -type f -name '*.pdb' -delete
}

make_zip() {
    archive=$1
    shift

    rm -f "$archive"

    if command -v zip >/dev/null 2>&1; then
        # Run from the output root so entries are relative to it.
        (cd "$OUTPUT_ROOT" && zip -q -r "$archive" "$@")
    elif command -v python3 >/dev/null 2>&1; then
        (cd "$OUTPUT_ROOT" && python3 - "$archive" "$@" <<'PY'
import os
import sys
import zipfile

archive = sys.argv[1]
roots = sys.argv[2:]
with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
    for root in roots:
        for dirpath, _, filenames in os.walk(root):
            for filename in filenames:
                path = os.path.join(dirpath, filename)
                zf.write(path, path)
PY
        )
    else
        die "creating $archive requires either 'zip' or 'python3'"
    fi

    echo "Created $archive"
}

publish_release() {
    archive=$1

    [ -f "$archive" ] || die "missing release archive: $archive"

    if ! command -v gh >/dev/null 2>&1; then
        die "-p requires the GitHub CLI (https://cli.github.com/)"
    fi
    if ! gh auth status >/dev/null 2>&1; then
        die "gh is not authenticated; run 'gh auth login' first"
    fi

    # Keep one rolling "latest" release so the release download URL stays
    # stable. Re-uploading with --clobber replaces the previous archive while
    # --latest keeps GitHub's /releases/latest redirect pointing here.
    echo "Publishing $archive to $RELEASE_REPO..."
    if gh release view latest --repo "$RELEASE_REPO" >/dev/null 2>&1; then
        gh release upload latest "$archive" \
            --repo "$RELEASE_REPO" \
            --clobber
        gh release edit latest \
            --repo "$RELEASE_REPO" \
            --notes "Automated newest build for macOS arm64, Linux x64 and Windows x64." \
            --latest
    else
        gh release create latest "$archive" \
            --repo "$RELEASE_REPO" \
            --title "Hexal latest builds" \
            --notes "Automated newest build for macOS arm64, Linux x64 and Windows x64." \
            --latest
    fi

    echo "Release updated:"
    echo "  https://github.com/$RELEASE_REPO/releases/latest/download/hexal-all-platforms.zip"
}

while getopts ":sapnr:o:h" option; do
    case "$option" in
        s) MODE="single" ;;
        a) MODE="all" ;;
        p) MODE="all"; PUBLISH=1 ;;
        r) RID=$OPTARG ;;
        o) OUTPUT_ROOT=$OPTARG ;;
        n) OBFUSCATE=0 ;;
        h) usage; exit 0 ;;
        :) die "option -$OPTARG requires an argument" ;;
        \?) die "unknown option -$OPTARG" ;;
    esac
done
shift $((OPTIND - 1))

if [ "$#" -ne 0 ]; then
    die "unexpected argument '$1'"
fi

if [ "$MODE" = "all" ] && [ -n "$RID" ]; then
    die "-a and -p publish every supported platform; -r is only valid with -s"
fi

if [ "$PUBLISH" -eq 1 ] && [ "$MODE" != "all" ]; then
    die "-p builds and uploads all platforms; it cannot be combined with -s"
fi

if [ "$MODE" = "aot" ]; then
    if [ -z "$RID" ]; then
        RID=$(host_rid)
    fi

    dotnet publish "$PROJECT" \
        -c "$CONFIGURATION" \
        -r "$RID" \
        -p:PublishAot=true \
        -p:Obfuscate=false
    exit 0
fi

if [ "$OBFUSCATE" -eq 1 ]; then
    echo "Restoring Obfuscar..."
    dotnet tool restore --tool-manifest "$SCRIPT_DIR/.config/dotnet-tools.json"
fi

mkdir -p "$OUTPUT_ROOT"
OUTPUT_ROOT=$(CDPATH= cd -- "$OUTPUT_ROOT" && pwd)

if [ "$MODE" = "single" ]; then
    if [ -z "$RID" ]; then
        RID=$(host_rid)
    fi

    publish_single_file "$RID" "$OUTPUT_ROOT/hexal-$RID"
    echo "Published $OUTPUT_ROOT/hexal-$RID"
    exit 0
fi

# -a: cross-platform, self-contained, single-file, non-AOT builds.
platforms="osx-arm64 linux-x64 win-x64"
folders=""

for platform in $platforms; do
    folder="hexal-$platform"
    echo "Publishing $platform..."
    publish_single_file "$platform" "$OUTPUT_ROOT/$folder"
    folders="$folders $folder"
done

# shellcheck disable=SC2086
make_zip "$OUTPUT_ROOT/hexal-all-platforms.zip" $folders
echo "Published:"
for folder in $folders; do
    echo "  $OUTPUT_ROOT/$folder"
done

if [ "$PUBLISH" -eq 1 ]; then
    publish_release "$OUTPUT_ROOT/hexal-all-platforms.zip"
fi
