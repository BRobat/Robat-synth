#!/usr/bin/env bash
# Builds the browser version into web/dist (static files for GitHub Pages).
#   web/build.sh            # build
#   web/build.sh --serve    # build and serve on http://localhost:8080
set -euo pipefail
cd "$(dirname "$0")/.."

rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo build -p robat-web --target wasm32-unknown-unknown --profile web

rm -rf web/dist
mkdir -p web/dist
cp web/www/* web/dist/
cp target/wasm32-unknown-unknown/web/robat_web.wasm web/dist/
if command -v wasm-opt >/dev/null; then
  wasm-opt -O3 web/dist/robat_web.wasm -o web/dist/robat_web.wasm
fi
# Bust the browser cache for the module and the worklet on each deploy.
version="$(git rev-parse --short HEAD 2>/dev/null || date +%s)"
sed -i.bak "s/robat-v1/robat-${version}/" web/dist/sw.js && rm web/dist/sw.js.bak
touch web/dist/.nojekyll
echo "built web/dist ($(du -h web/dist/robat_web.wasm | cut -f1) wasm)"

if [[ "${1:-}" == "--serve" ]]; then
  cd web/dist && python3 -m http.server 8080
fi
