#!/usr/bin/env bash
# Run after this image is deployed. This script does not deploy.
set -euo pipefail
cd "$(dirname "$0")/../.."
STATE="$(cd "$(dirname "$0")" && pwd)/.state"
# shellcheck disable=SC1091
source "$STATE/cluster.env"
: "${API_URL:?}"
: "${ALB_URL:?}"
: "${BASE_DOMAIN:?}"
: "${OPERATOR_TOKEN:?}"
export REACTOR_HOME="${REACTOR_HOME:-$(mktemp -d)}"
CLI="${REACTOR_CLI:-$PWD/target/debug/reactor-cli}"
if [[ ! -x "$CLI" ]]; then
  cargo build -p reactor-cli
fi

curl -sf "$API_URL/health" >/dev/null
curl -sf "$ALB_URL/health" >/dev/null
curl -sf "$ALB_URL/console" | grep -q '<title>Reactor</title>'

needs="$(curl -sf "$ALB_URL/console/v1/setup" | python3 -c 'import json,sys; print("yes" if json.load(sys.stdin)["needs_setup"] else "no")')"
if [[ ! -f "$STATE/console-password" ]]; then
  umask 077
  openssl rand -hex 16 >"$STATE/console-password"
fi
password="$(cat "$STATE/console-password")"
if [[ "$needs" == yes ]]; then
  "$CLI" setup --url "$ALB_URL" --cluster AWS --email admin@reactor.dev --name Admin --password "$password"
else
  "$CLI" login --url "$ALB_URL" --email admin@reactor.dev --password "$password"
fi

retry() {
  local tries="$1"
  shift
  local i
  for i in $(seq 1 "$tries"); do
    if "$@"; then
      return 0
    fi
    sleep 3
  done
  return 1
}

work="$(mktemp -d)"
mkdir -p "$work"
cp -R examples/todos/functions examples/todos/site "$work/"
(
  cd "$work"
  "$CLI" projects create aws-todos --link
  ref="$(python3 -c 'import pathlib,re; text=pathlib.Path("reactor.toml").read_text(); print(re.search(r"ref = \"([a-z0-9]+)\"", text).group(1))')"
  key="$(cat .reactor/service_key)"
  printf '%s\n' "$ref" >"$STATE/project-ref"
  put="$(curl -sf -X POST "$ALB_URL/storage/v1/object/presign" \
    -H "Authorization: Bearer $key" \
    -H 'content-type: application/json' \
    -d '{"bucket":"probe","key":"hello.txt","method":"PUT"}')"
  put_url="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["url"])' "$put")"
  curl -sf -X PUT -H 'content-type: text/plain' --data 'hello' "$put_url" >/dev/null
  get="$(curl -sf -X POST "$ALB_URL/storage/v1/object/presign" \
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
  token="$(python3 -c 'import json,os,pathlib; print(json.loads(pathlib.Path(os.environ["REACTOR_HOME"], "credentials.json").read_text())["console_token"])')"
  thread="$(curl -sf -X POST "$ALB_URL/console/v1/agent/threads" \
    -H "Authorization: Bearer $token" \
    -H 'content-type: application/json' \
    -d "{\"project_ref\":\"$ref\"}")"
  thread_id="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$thread")"
  code="$(curl -s -o "$STATE/agent-post.json" -w '%{http_code}' -X POST "$ALB_URL/console/v1/agent/threads/${thread_id}/messages" \
    -H "Authorization: Bearer $token" \
    -H 'content-type: application/json' \
    -d '{"text":"Reply with the single word pong."}')"
  [[ "$code" == 202 ]]
  assistant=0
  for _ in $(seq 1 36); do
    curl -sf "$ALB_URL/console/v1/agent/threads/${thread_id}" -H "Authorization: Bearer $token" >"$STATE/agent-thread.json"
    if python3 -c 'import json,sys; body=json.load(open(sys.argv[1])); sys.exit(0 if any(m.get("role")=="assistant" and (m.get("content") or m.get("text")) for m in body.get("messages", [])) else 1)' "$STATE/agent-thread.json"; then
      assistant=1
      break
    fi
    sleep 5
  done
  [[ "$assistant" == 1 ]]
)

probe="$(mktemp -d)"
mkdir -p "$probe/functions/probe" "$probe/site"
cat >"$probe/functions/probe/index.ts" <<'EOF'
process.stdout.write(JSON.stringify({ secret: process.env.API_SECRET ?? null, site: process.env.TOKEN ?? null }))
EOF
printf 'from-aws-a\n' >"$probe/site/index.html"
(
  cd "$probe"
  "$CLI" projects create aws-a --link
  ref="$(python3 -c 'import pathlib,re; text=pathlib.Path("reactor.toml").read_text(); print(re.search(r"ref = \"([a-z0-9]+)\"", text).group(1))')"
  printf '%s\n' "$ref" >"$STATE/ref-a"
)
ref_a="$(cat "$STATE/ref-a")"
token="$(python3 -c 'import json,os,pathlib; print(json.loads(pathlib.Path(os.environ["REACTOR_HOME"], "credentials.json").read_text())["console_token"])')"
curl -sf -X POST "$ALB_URL/console/v1/projects/${ref_a}/functions/probe/env" \
  -H "Authorization: Bearer $token" \
  -H 'content-type: application/json' \
  -d '{"key":"API_SECRET","value":"fn-only"}' >/dev/null
(
  cd "$probe"
  "$CLI" deploy
)
key_a="$(cat "$probe/.reactor/service_key")"
retry 20 curl -sf -X POST "$ALB_URL/fn/v1/probe" \
  -H "Authorization: Bearer $key_a" \
  -H 'content-type: application/json' \
  -d '{}' | tee "$STATE/probe.json"
grep -q 'fn-only' "$STATE/probe.json"
if grep -q 'site-only' "$STATE/probe.json"; then
  echo "function saw site env" >&2
  exit 1
fi

other="$(mktemp -d)"
mkdir -p "$other/site"
printf 'from-aws-b\n' >"$other/site/index.html"
(
  cd "$other"
  "$CLI" projects create aws-b --link
  "$CLI" deploy
  ref="$(python3 -c 'import pathlib,re; text=pathlib.Path("reactor.toml").read_text(); print(re.search(r"ref = \"([a-z0-9]+)\"", text).group(1))')"
  printf '%s\n' "$ref" >"$STATE/ref-b"
)
ref_b="$(cat "$STATE/ref-b")"
retry 20 curl -sf -H "Host: ${ref_a}.${BASE_DOMAIN}" "$ALB_URL/" | tee "$STATE/site-a.txt"
grep -q 'from-aws-a' "$STATE/site-a.txt"
retry 20 curl -sf -H "Host: ${ref_b}.${BASE_DOMAIN}" "$ALB_URL/" | tee "$STATE/site-b.txt"
grep -q 'from-aws-b' "$STATE/site-b.txt"
if grep -q 'from-aws-b' "$STATE/site-a.txt"; then
  echo "host ${ref_a} served project b" >&2
  exit 1
fi

nodework="$(mktemp -d)"
cat >"$nodework/server.js" <<'EOF'
const http = require("http");
http.createServer((req, res) => {
  res.end(JSON.stringify({ token: process.env.TOKEN || "", path: req.url }));
}).listen(Number(process.env.PORT), "127.0.0.1");
EOF
(
  cd "$nodework"
  "$CLI" projects create aws-node --link
  "$CLI" sites env set TOKEN site-only
)
node_ref="$(python3 -c 'import pathlib,re; text=pathlib.Path("'"$nodework"'/reactor.toml").read_text(); print(re.search(r"ref = \"([a-z0-9]+)\"", text).group(1))')"
node_key="$(cat "$nodework/.reactor/service_key")"
deploy_id="$(curl -sf -X POST "$ALB_URL/sites/v1/deployments" -H "Authorization: Bearer $node_key" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"
curl -sf -X PUT "$ALB_URL/sites/v1/deployments/${deploy_id}/files/server.js" \
  -H "Authorization: Bearer $node_key" \
  --data-binary @"$nodework/server.js" >/dev/null
curl -sf -X POST "$ALB_URL/sites/v1/deployments/${deploy_id}/finish" \
  -H "Authorization: Bearer $node_key" \
  -H 'content-type: application/json' \
  -d '{"command":"node server.js"}' >/dev/null
retry 20 curl -sf -H "Host: ${node_ref}.${BASE_DOMAIN}" "$ALB_URL/ping" | tee "$STATE/node.txt"
grep -q 'site-only' "$STATE/node.txt"

curl -sf -X POST "$ALB_URL/sites/v1/domains" \
  -H "Authorization: Bearer $node_key" \
  -H 'content-type: application/json' \
  -d '{"host":"custom.example"}' >"$STATE/domain.json"
curl -sf -X POST "$ALB_URL/platform/v1/domains/custom.example/verify" \
  -H "Authorization: Bearer $OPERATOR_TOKEN" >/dev/null
matched=0
for _ in $(seq 1 10); do
  if curl -sf -H "Host: custom.example" "$ALB_URL/ping" | grep -q 'site-only'; then
    matched=1
    break
  fi
  sleep 3
done
[[ "$matched" == 1 ]]

curl -sf -X POST "$ALB_URL/fn/v1/_internal/cron" \
  -H "Authorization: Bearer $OPERATOR_TOKEN" \
  -H 'content-type: application/json' \
  -d '{}' | tee "$STATE/cron.json"
grep -q '"ran"' "$STATE/cron.json"
echo "prove ok"
