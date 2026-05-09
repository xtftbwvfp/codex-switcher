#!/usr/bin/env bash
# GLM Coding Plan smoke test for the chat_completions Relay translator.
#
# Run AFTER you've rebuilt + restarted the Codex Switcher app on the bundled
# version that includes the relay_translate module.

set -euo pipefail

ACCOUNTS_JSON="$HOME/.codex-switcher/accounts.json"
PROXY_BASE="${PROXY_BASE:-http://127.0.0.1:18080}"

cat <<'EOF'
=========================================================
GLM Coding Plan smoke test for codex-switcher relay translator
=========================================================

Step 1 — manually edit your GLM account in:
  ~/.codex-switcher/accounts.json

The Relay account entry should look like (only these fields are critical;
keep other fields such as id/name/created_at as-is):

  "kind": "relay",
  "relay_base_url": "https://open.bigmodel.cn/api/coding/paas/v4",
  "relay_protocol": "chat_completions",
  "auth_json": { "tokens": { "access_token": "5d9a572459d543ccae678168756355e9.FpsmvbDNXWIexvHl", ... } },
  "relay_model_map": { ...keep your current value... }

After editing, RESTART the codex-switcher app (so it reloads accounts.json).

Step 2 — make sure this account is currently selected (UI: "切到此号"),
and that the proxy is enabled (default port 18080).

Step 3 — when ready, press ENTER and we'll send a tiny request through the
proxy and watch the SSE stream come back.
EOF

read -p "Press ENTER when ready (or Ctrl-C to abort)... " _

if [[ ! -f "$ACCOUNTS_JSON" ]]; then
  echo "[error] $ACCOUNTS_JSON does not exist — run codex-switcher once first."
  exit 1
fi

if ! grep -q "chat_completions" "$ACCOUNTS_JSON"; then
  echo "[warn] No 'chat_completions' string found in $ACCOUNTS_JSON."
  echo "       Did you remember to set relay_protocol on the GLM entry?"
  echo "       Continuing anyway in case you used quoting differently."
fi

# Tiny smoke request: stream a one-token completion. Must use codex
# /v1/responses shape — the proxy will translate to /chat/completions.
PAYLOAD='{
  "model": "gpt-5",
  "instructions": "You only ever reply with the single word: pong.",
  "input": "ping",
  "stream": true
}'

echo
echo "=== POST $PROXY_BASE/v1/responses ==="
echo "(if this hangs forever, the proxy isn't running on $PROXY_BASE)"
echo

# 30s timeout — GLM can take a few seconds to start streaming
RESP=$(curl -sS --max-time 30 -N \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer dummy-codex-cli-bearer' \
  --data "$PAYLOAD" \
  "$PROXY_BASE/v1/responses" || true)

echo "--- RAW RESPONSE (truncated to 4 KB) ---"
printf '%s\n' "$RESP" | head -c 4096
echo
echo "--- END ---"

if echo "$RESP" | grep -q '"type":"response.created"' && \
   echo "$RESP" | grep -q '"type":"response.completed"'; then
  echo
  echo "[OK] saw response.created + response.completed in the stream."
  exit 0
fi

if echo "$RESP" | grep -qi 'error'; then
  echo
  echo "[FAIL] response contains an error. See RAW RESPONSE above."
  exit 2
fi

echo
echo "[?]  Couldn't find both response.created and response.completed events."
echo "     Either the upstream errored, the proxy didn't translate, or the"
echo "     account isn't currently selected. Inspect the raw response above."
exit 3
