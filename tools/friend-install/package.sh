#!/bin/bash
# Zips a Windows build for the installer beside this script, with DXC from the Windows SDK.
#   package.sh TAG RELEASE_DIR [GRAPHICS_FILE]
# REDIST defaults to the SDK's redistributables on a Windows disk mounted at /mnt/win.
set -eu
tag=$1 release=$2 graphics=${3:-}
here=$(cd "$(dirname "$0")" && pwd)
redist=${REDIST:-"/mnt/win/Program Files (x86)/Windows Kits/10/Redist/D3D/x64"}
out=${OUT:-$HOME/Projects/builds/friend-$tag.zip}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$release/horizon-client.exe" "$release/horizon-ctl.exe" "$redist/dxcompiler.dll" "$redist/dxil.dll" \
    "$here/install.ps1" "$here/Install Horizon build.cmd" "$work/"
[ -z "$graphics" ] || cp "$graphics" "$work/horizon-graphics.txt"
echo "$tag" > "$work/tag.txt"
rm -f "$out"
(cd "$work" && zip -q -9 "$out" ./*)
ls -la "$out"
