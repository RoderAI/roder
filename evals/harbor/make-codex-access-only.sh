#!/bin/bash
#
# Derive an access-token-only Codex auth file for concurrent local Harbor runs.
#
# Concurrent Terminal-Bench trials each upload the same auth file into an
# isolated container. If that file carries a refresh token and the access token
# is expired, every container tries to refresh at once; Codex rotates (and
# invalidates) the shared refresh token on first use, so all but one container
# fail with `refresh_token_reused`. Worse, ChatGPT treats concurrent reuse as a
# breach and can revoke the whole token family, which only `codex login` undoes.
#
# The fix is to hand the containers a *valid access token only* (no refresh
# token), so none of them attempt a refresh. This requires a currently-valid
# access token, so re-authenticate Codex first:
#
#   codex login          # interactive; refreshes ~/.codex/auth.json
#   ./evals/harbor/make-codex-access-only.sh
#
# Then run Harbor with:
#   RODER_HARBOR_AUTH_FILE="$PWD/evals/harbor/artifacts/codex-access-only.json" harbor run ...
#
# NOTE: an access-token-only file is a LOCAL-DEVELOPMENT deviation. It is not a
# leaderboard-valid auth mode; keep it out of any submission-track run.
#
# The default source is ~/.codex/auth.json (what `codex login` writes). Both the
# Codex-native shape ({"tokens": {"access_token", "refresh_token", ...}}) and the
# Roder shape ({"access", "refresh", "expires", ...}) are accepted.
set -euo pipefail

SRC="${1:-$HOME/.codex/auth.json}"
DST="${2:-$(cd "$(dirname "$0")" && pwd)/artifacts/codex-access-only.json}"

mkdir -p "$(dirname "$DST")"

python3 - "$SRC" "$DST" <<'PY'
import base64, json, os, sys, time

src, dst = sys.argv[1], sys.argv[2]
with open(src) as handle:
    auth = json.load(handle)

# Accept both the Codex-native shape and the Roder shape.
tokens = auth.get("tokens") if isinstance(auth.get("tokens"), dict) else {}
access = tokens.get("access_token") or auth.get("access")
account_id = tokens.get("account_id") or auth.get("account_id") or ""
if not isinstance(access, str) or not access:
    raise SystemExit(f"{src}: no access token to isolate (run `codex login`)")


def jwt_exp_ms(jwt):
    try:
        payload = jwt.split(".")[1]
        payload += "=" * (-len(payload) % 4)
        exp = json.loads(base64.urlsafe_b64decode(payload)).get("exp")
        return int(exp) * 1000 if isinstance(exp, (int, float)) else None
    except Exception:
        return None


# Prefer an explicit `expires` (Roder shape, ms); else read the JWT `exp` claim.
expires = auth.get("expires")
if not isinstance(expires, int):
    expires = jwt_exp_ms(access)

now_ms = int(time.time() * 1000)
if isinstance(expires, int) and expires <= now_ms:
    stale_min = round((now_ms - expires) / 60000, 1)
    raise SystemExit(
        f"{src}: access token expired {stale_min} min ago; run `codex login` "
        "to refresh before regenerating the access-only file"
    )

# Keep only the fields the container needs to authenticate with the access
# token directly; drop the refresh token so no container attempts a refresh.
out = {
    "type": auth.get("type", "bearer"),
    "access": access,
    "account_id": account_id,
    "expires": expires if isinstance(expires, int) else now_ms,
    "refresh": "",
}
with open(dst, "w") as handle:
    json.dump(out, handle)
os.chmod(dst, 0o600)

if isinstance(expires, int):
    valid = f"~{round((expires - now_ms) / 3600000, 1)}h"
else:
    valid = "unknown ttl"
print(f"wrote {dst} (access-token-only, valid {valid})")
print(
    "NOTE: revocation is only detectable via a live API call; if a run returns "
    "401/'refresh token was revoked', run `codex login` and regenerate."
)
PY
