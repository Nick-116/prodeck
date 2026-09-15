#!/bin/bash
# Build the ProDeck Docker image.
#
#   bash scripts/build-docker.sh           # latest tag
#   bash scripts/build-docker.sh 1.2.3     # version tag
#
# Produces:  prodeck:latest  (and prodeck:<version> when given)
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR"

VERSION="$(grep '^version' src-tauri/Cargo.toml | head -1 | sed 's/.*= *"//' | sed 's/"//')"
TAG="${1:-latest}"

echo "▸ Building ProDeck $VERSION as prodeck:$TAG"

docker build \
  --tag "prodeck:$TAG" \
  --tag "prodeck:$VERSION" \
  --file Dockerfile \
  .

echo ""
echo "✓ Done  (prodeck:$TAG and prodeck:$VERSION)"
echo ""
echo "Run:   docker compose up -d"
echo "Open:  http://localhost:4000"
