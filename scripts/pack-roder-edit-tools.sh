#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "--dry-run" ]]; then
  echo "refusing to run without --dry-run" >&2
  exit 2
fi

if [[ "${RODER_NPM_PUBLISH:-}" == "1" ]]; then
  echo "RODER_NPM_PUBLISH=1 is not supported by the dry-run pack script" >&2
  exit 2
fi

cargo test -p roder-edit-core
(
  cd packages/edit-tools
  pnpm run typecheck
  pnpm test
  pnpm pack --dry-run
  tmpdir=$(mktemp -d)
  tarball=$(pnpm pack --pack-destination "$tmpdir" | tail -n 1)
  mkdir -p "$tmpdir/smoke"
  cat > "$tmpdir/smoke/package.json" <<'JSON'
{"type":"module"}
JSON
  (cd "$tmpdir/smoke" && pnpm add "$tarball" >/dev/null && node -e "import('@roderai/edit-tools').then((m)=>{ if (!m.createMemoryEditWorkspace) process.exit(1) })")
  rm -rf "$tmpdir"
)
