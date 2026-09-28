#!/usr/bin/env bash
set -euo pipefail
V2="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$V2/.." && pwd)"
DEST="${REACTOR_RELEASE_DIR:-/Users/cdelconde/Dev/Github/Reactor/reactor}"
JS_DEST="${REACTOR_JS_DIR:-/Users/cdelconde/Dev/Github/Reactor/reactor-js}"
SWIFT_DEST="${REACTOR_SWIFT_DIR:-/Users/cdelconde/Dev/Github/Reactor/reactor-swift}"
KOTLIN_DEST="${REACTOR_KOTLIN_DIR:-/Users/cdelconde/Dev/Github/Reactor/reactor-kotlin}"
BREW_DEST="${REACTOR_HOMEBREW_DIR:-/Users/cdelconde/Dev/Github/Reactor/homebrew-reactor}"

if [[ ! -d "$DEST/.git" ]]; then
  echo "release repo not found at $DEST" >&2
  exit 1
fi

ensure_repo() {
  local dest="$1"
  mkdir -p "$dest"
  if [[ ! -d "$dest/.git" ]]; then
    git -C "$dest" init -b main
  fi
}

rsync -a --delete \
  --exclude .git \
  --exclude target \
  --exclude node_modules \
  --exclude .data \
  --exclude '.env' \
  --exclude '.env.*' \
  --exclude '*.pem' \
  --exclude reactor.toml \
  --exclude .reactor \
  --exclude 'websites/docs/dist' \
  --exclude 'websites/docs/.astro' \
  "$V2/" "$DEST/"

mkdir -p "$ROOT/.github/workflows"
cp "$V2/.github/workflows/"* "$ROOT/.github/workflows/"

ensure_repo "$JS_DEST"
rsync -a --delete \
  --exclude .git \
  --exclude node_modules \
  "$V2/sdks/js/" "$JS_DEST/"
cp "$V2/LICENSE" "$JS_DEST/LICENSE"

ensure_repo "$SWIFT_DEST"
rsync -a --delete \
  --exclude .git \
  --exclude .build \
  "$V2/sdks/swift/" "$SWIFT_DEST/"
cp "$V2/LICENSE" "$SWIFT_DEST/LICENSE"

ensure_repo "$KOTLIN_DEST"
rsync -a --delete \
  --exclude .git \
  --exclude build \
  --exclude .gradle \
  "$V2/sdks/kotlin/reactor-client/" "$KOTLIN_DEST/reactor-client/"
rsync -a --delete \
  --exclude .git \
  "$V2/sdks/kotlin/gradle/" "$KOTLIN_DEST/gradle/"
cp "$V2/sdks/kotlin/gradlew" "$KOTLIN_DEST/gradlew"
cp "$V2/sdks/kotlin/gradle.properties" "$KOTLIN_DEST/gradle.properties"
cp "$V2/sdks/kotlin/.gitignore" "$KOTLIN_DEST/.gitignore"
cp "$V2/sdks/kotlin/README.md" "$KOTLIN_DEST/README.md"
cp "$V2/deploy/publish/kotlin/build.gradle.kts" "$KOTLIN_DEST/build.gradle.kts"
cp "$V2/deploy/publish/kotlin/settings.gradle.kts" "$KOTLIN_DEST/settings.gradle.kts"
mkdir -p "$KOTLIN_DEST/.github/workflows"
cp "$V2/deploy/publish/kotlin/release.yml" "$KOTLIN_DEST/.github/workflows/release.yml"
cp "$V2/LICENSE" "$KOTLIN_DEST/LICENSE"
chmod +x "$KOTLIN_DEST/gradlew"

ensure_repo "$BREW_DEST"
mkdir -p "$BREW_DEST/Formula"
cp "$V2/deploy/publish/homebrew/Formula/reactor.rb" "$BREW_DEST/Formula/reactor.rb"
cp "$V2/deploy/publish/homebrew/README.md" "$BREW_DEST/README.md"
cp "$V2/LICENSE" "$BREW_DEST/LICENSE"

echo "synced $V2 -> $DEST"
echo "workflows -> $ROOT/.github/workflows"
echo "js -> $JS_DEST"
echo "swift -> $SWIFT_DEST"
echo "kotlin -> $KOTLIN_DEST"
echo "homebrew -> $BREW_DEST"
