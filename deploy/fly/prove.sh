#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

APP=reactor-v2
DB=reactor-v2-db
URL=https://reactor-v2.fly.dev
STATE="$(cd "$(dirname "$0")" && pwd)/.state"
# shellcheck disable=SC1091
source "$STATE/cluster.env"
export REACTOR_HOME="${REACTOR_HOME:-$(mktemp -d)}"
CLI="${REACTOR_CLI:-$PWD/target/debug/reactor-cli}"

if [[ ! -x "$CLI" ]]; then
  cargo build -p reactor-cli
fi

curl -sf "$URL/health" >/dev/null
curl -sf "$URL/console" | grep -q '<title>Reactor</title>'

needs="$(curl -sf "$URL/console/v1/setup" | python3 -c 'import json,sys; print("yes" if json.load(sys.stdin)["needs_setup"] else "no")')"
if [[ ! -f "$STATE/console-password" ]]; then
  umask 077
  openssl rand -hex 16 >"$STATE/console-password"
fi
password="$(cat "$STATE/console-password")"
if [[ "$needs" == yes ]]; then
  "$CLI" setup --url "$URL" --cluster Fly --email admin@reactor.dev --name Admin --password "$password"
else
  "$CLI" login --url "$URL" --email admin@reactor.dev --password "$password"
fi

work="$(mktemp -d)"
mkdir -p "$work"
cp -R examples/todos/functions examples/todos/site "$work/"
(
  cd "$work"
  "$CLI" projects create fly-todos --link
  ref="$(python3 -c 'import pathlib,re; text=pathlib.Path("reactor.toml").read_text(); print(re.search(r"ref = \"([a-z0-9]+)\"", text).group(1))')"
  key="$(cat .reactor/service_key)"
  printf '%s\n' "$ref" >"$STATE/project-ref"
  put="$(curl -sf -X POST "$URL/storage/v1/object/presign" \
    -H "Authorization: Bearer $key" \
    -H 'content-type: application/json' \
    -d '{"bucket":"probe","key":"hello.txt","method":"PUT"}')"
  put_url="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["url"])' "$put")"
  curl -sf -X PUT -H 'content-type: text/plain' --data 'hello' "$put_url" >/dev/null
  get="$(curl -sf -X POST "$URL/storage/v1/object/presign" \
    -H "Authorization: Bearer $key" \
    -H 'content-type: application/json' \
    -d '{"bucket":"probe","key":"hello.txt","method":"GET"}')"
  get_url="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["url"])' "$get")"
  curl -sf "$get_url" | grep -q hello
  "$CLI" deploy
  "$CLI" cluster | tee "$STATE/cluster.txt"
  grep -q $'postgres\tup' "$STATE/cluster.txt"
  grep -q $'postgrest\tup' "$STATE/cluster.txt"
  "$CLI" db tables | tee "$STATE/tables.txt"
  grep -q todos "$STATE/tables.txt"
  curl -sf -X POST "$URL/fn/v1/ping" \
    -H "Authorization: Bearer $key" \
    -H 'content-type: application/json' \
    -d '{"marker":"fly"}' | tee "$STATE/ping.json"
  grep -q '"ok":true' "$STATE/ping.json"
  token="$(python3 -c 'import json,os,pathlib; print(json.loads(pathlib.Path(os.environ["REACTOR_HOME"], "credentials.json").read_text())["console_token"])')"
  thread="$(curl -sf -X POST "$URL/console/v1/agent/threads" \
    -H "Authorization: Bearer $token" \
    -H 'content-type: application/json' \
    -d "{\"project_ref\":\"$ref\"}")"
  thread_id="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$thread")"
  code="$(curl -s -o "$STATE/agent-post.json" -w '%{http_code}' -X POST "$URL/console/v1/agent/threads/${thread_id}/messages" \
    -H "Authorization: Bearer $token" \
    -H 'content-type: application/json' \
    -d '{"text":"Reply with the single word pong."}')"
  [[ "$code" == 202 ]]
  assistant=0
  for _ in $(seq 1 36); do
    curl -sf "$URL/console/v1/agent/threads/${thread_id}" -H "Authorization: Bearer $token" >"$STATE/agent-thread.json"
    if python3 -c 'import json,sys; body=json.load(open(sys.argv[1])); sys.exit(0 if any(m.get("role")=="assistant" and (m.get("content") or m.get("text")) for m in body.get("messages", [])) else 1)' "$STATE/agent-thread.json"; then
      assistant=1
      break
    fi
    sleep 5
  done
  [[ "$assistant" == 1 ]]
  fly proxy 19080:8000 -a "$APP" >"$STATE/proxy.log" 2>&1 &
  proxy_pid=$!
  trap 'kill "$proxy_pid" >/dev/null 2>&1 || true' EXIT
  site=0
  for _ in $(seq 1 20); do
    if curl -sf -H "Host: ${ref}.reactor-v2.fly.dev" http://127.0.0.1:19080/ | grep -q '<h1>Todos</h1>'; then
      site=1
      break
    fi
    sleep 1
  done
  [[ "$site" == 1 ]]

  other="$(mktemp -d)"
  mkdir -p "$other/site"
  printf 'from-fly-b\n' >"$other/site/index.html"
  (
    cd "$other"
    "$CLI" projects create fly-other --link
    other_ref="$(python3 -c 'import pathlib,re; text=pathlib.Path("reactor.toml").read_text(); print(re.search(r"ref = \"([a-z0-9]+)\"", text).group(1))')"
    printf '%s\n' "$other_ref" >"$STATE/other-ref"
    "$CLI" deploy
  )
  other_ref="$(cat "$STATE/other-ref")"
  other_body=0
  for _ in $(seq 1 20); do
    if curl -sf -H "Host: ${other_ref}.reactor-v2.fly.dev" http://127.0.0.1:19080/ | grep -q 'from-fly-b'; then
      other_body=1
      break
    fi
    sleep 1
  done
  [[ "$other_body" == 1 ]]
  todos="$(curl -sf -H "Host: ${ref}.reactor-v2.fly.dev" http://127.0.0.1:19080/)"
  [[ "$todos" == *'<h1>Todos</h1>'* ]]
  [[ "$todos" != *from-fly-b* ]]

  nodework="$(mktemp -d)"
  mkdir -p "$nodework/functions/probe"
  cat >"$nodework/functions/probe/index.ts" <<'EOF'
process.stdout.write(JSON.stringify({ secret: process.env.API_SECRET ?? null, site: process.env.TOKEN ?? null }))
EOF
  cat >"$nodework/server.js" <<'EOF'
const http = require("http");
http.createServer((req, res) => {
  res.end(JSON.stringify({ token: process.env.TOKEN || "", path: req.url }));
}).listen(Number(process.env.PORT), "127.0.0.1");
EOF
  (
    cd "$nodework"
    "$CLI" projects create fly-node --link
    node_ref="$(python3 -c 'import pathlib,re; text=pathlib.Path("reactor.toml").read_text(); print(re.search(r"ref = \"([a-z0-9]+)\"", text).group(1))')"
    printf '%s\n' "$node_ref" >"$STATE/node-ref"
    "$CLI" sites env set TOKEN site-only
    "$CLI" deploy
  )
  node_ref="$(cat "$STATE/node-ref")"
  node_key="$(cat "$nodework/.reactor/service_key")"
  deploy_id="$(curl -sf -X POST "$URL/sites/v1/deployments" -H "Authorization: Bearer $node_key" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"
  curl -sf -X PUT "$URL/sites/v1/deployments/${deploy_id}/files/server.js" \
    -H "Authorization: Bearer $node_key" \
    --data-binary @"$nodework/server.js" >/dev/null
  curl -sf -X POST "$URL/sites/v1/deployments/${deploy_id}/finish" \
    -H "Authorization: Bearer $node_key" \
    -H 'content-type: application/json' \
    -d '{"command":"node server.js"}' >/dev/null
  node_echo=0
  for _ in $(seq 1 20); do
    if curl -sf -H "Host: ${node_ref}.reactor-v2.fly.dev" http://127.0.0.1:19080/ping | grep -q 'site-only'; then
      node_echo=1
      break
    fi
    sleep 1
  done
  [[ "$node_echo" == 1 ]]
  console_token="$(python3 -c 'import json,os,pathlib; print(json.loads(pathlib.Path(os.environ["REACTOR_HOME"], "credentials.json").read_text())["console_token"])')"
  curl -sf -X POST "$URL/console/v1/projects/${node_ref}/functions/probe/env" \
    -H "Authorization: Bearer $console_token" \
    -H 'content-type: application/json' \
    -d '{"key":"API_SECRET","value":"fn-only"}' >/dev/null
  curl -sf -X POST "$URL/fn/v1/probe" \
    -H "Authorization: Bearer $node_key" \
    -H 'content-type: application/json' \
    -d '{}' | tee "$STATE/probe.json"
  grep -q 'fn-only' "$STATE/probe.json"
  grep -vq 'site-only' "$STATE/probe.json"
)

curl -sf -X POST "$URL/fn/v1/_internal/cron" \
  -H "Authorization: Bearer $OPERATOR_TOKEN" \
  -H 'content-type: application/json' \
  -d '{}' | tee "$STATE/cron.json"
grep -q '"ran"' "$STATE/cron.json"
started="$(fly machine list -a "$APP" --json | python3 -c '
import json, sys
machines = json.load(sys.stdin)
print(sum(1 for m in machines if (m.get("state") or m.get("State")) == "started"))
')"
[[ "$started" -ge 2 ]]
volume_id="$(fly volumes list -a "$DB" --json | python3 -c 'import json,sys; vols=json.load(sys.stdin); print(vols[0].get("id") or vols[0].get("ID"))')"
fly volumes snapshots create "$volume_id" -a "$DB"
echo "prove ok"
