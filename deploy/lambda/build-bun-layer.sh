#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
rm -rf bun-layer
mkdir -p bun-layer/bin
tmp="$(mktemp -d)"
curl -fsSL "https://github.com/oven-sh/bun/releases/latest/download/bun-linux-aarch64.zip" -o "$tmp/bun.zip"
unzip -q "$tmp/bun.zip" -d "$tmp"
install -m 0755 "$tmp"/bun-linux-aarch64/bun bun-layer/bin/bun
rm -rf "$tmp"
echo "bun layer ready at bun-layer/bin/bun"
