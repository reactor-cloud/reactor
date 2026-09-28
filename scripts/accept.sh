#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p .data/dns
touch .data/dns/stub.json

echo "unit tests"
cargo test --workspace --exclude reactor-accept

echo "cli"
cargo build -p reactor-cli -p reactor-builder

COMPOSE=(docker compose -p reactor-v2 -f deploy/compose/compose.yaml)
"${COMPOSE[@]}" up -d --build

for _ in $(seq 1 60); do
  if curl -sf http://127.0.0.1:18000/health >/dev/null && curl -sf http://127.0.0.1:18001/health >/dev/null; then
    break
  fi
  sleep 2
done
curl -sf http://127.0.0.1:18000/health >/dev/null

export REACTOR_ACCEPT=1
export REACTOR_OPERATOR_TOKEN=dev-operator-token
export REACTOR_ACCEPT_DATABASE_URL=postgres://reactor:reactor@127.0.0.1:5440/reactor
export REACTOR_ACCEPT_DEDICATED_URL=postgres://reactor:reactor@127.0.0.1:5441/reactor
export REACTOR_ACCEPT_AUTHENTICATOR_URL=postgres://authenticator:authenticator@127.0.0.1:5440/reactor
export REACTOR_ACCEPT_PATCH_DATABASE_URL=postgres://reactor:reactor@postgres-dedicated:5432/reactor
export REACTOR_DNS_STUB_HOST="$PWD/.data/dns/stub.json"
export REACTOR_CLI="$PWD/target/debug/reactor-cli"
export REACTOR_BUILDER="$PWD/target/debug/reactor-builder"

echo "gates"
cargo test -p reactor-accept -- --test-threads=1 --nocapture

echo "sam validate"
if ! command -v sam >/dev/null 2>&1; then
  echo "aws sam cli is required. Install it from https://docs.aws.amazon.com/serverless-application-model/latest/developerguide/install-sam-cli.html" >&2
  exit 1
fi
sam validate --template-file deploy/lambda/template.yaml --lint

echo "lambda runtime emulator"
docker rm -f reactor-v2-rie >/dev/null 2>&1 || true
docker run -d --name reactor-v2-rie \
  --entrypoint /usr/local/bin/aws-lambda-rie \
  -p 127.0.0.1:19090:8080 \
  --add-host=host.docker.internal:host-gateway \
  -e REACTOR_HANDLER=platform \
  -e REACTOR_RUNTIME__MODE=lambda \
  -e REACTOR_DATABASE__URL=postgres://reactor:reactor@host.docker.internal:5440/reactor \
  -e REACTOR_AUTH__PROVIDER=internal \
  -e REACTOR_AUTH__OPERATOR_TOKEN=dev-operator-token \
  -e REACTOR_AUTH__JWT_PRIVATE_PEM_FILE=/keys/private.pem \
  -e REACTOR_STORAGE__BACKEND=fs \
  -e REACTOR_SQL_DIR=/app/sql \
  -v reactor-v2_keys:/keys:ro \
  reactor-v2:local \
  /usr/local/bin/reactor-server
for _ in $(seq 1 30); do
  if curl -sf -X POST "http://127.0.0.1:19090/2015-03-31/functions/function/invocations" \
    -H 'content-type: application/json' \
    -d '{"version":"2.0","rawPath":"/health","requestContext":{"http":{"method":"GET","path":"/health"}},"headers":{"host":"localhost"}}' \
    | grep -q '"statusCode":200'; then
    docker rm -f reactor-v2-rie >/dev/null
    echo "sdk project"
    created="$(curl -sf -X POST http://127.0.0.1:18000/platform/v1/projects \
      -H "authorization: Bearer ${REACTOR_OPERATOR_TOKEN}" \
      -H 'content-type: application/json' \
      -d '{}')"
    ref="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["ref"])' "$created")"
    anon="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["anon_key"])' "$created")"
    service="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["service_key"])' "$created")"
    python3 - <<'PY'
import zipfile
from pathlib import Path
src = Path("examples/todos/functions/ping/index.ts").read_bytes()
out = Path(".data/ping.zip")
with zipfile.ZipFile(out, "w") as archive:
    archive.writestr("index.ts", src)
PY
    curl -sf -X POST "http://127.0.0.1:18000/fn/v1/_admin/functions/ping" \
      -H "host: ${ref}.apps.localhost:18000" \
      -H "authorization: Bearer ${service}" \
      -H 'content-type: application/zip' \
      --data-binary @.data/ping.zip >/dev/null
    cat > .data/sdk.env <<EOF
REACTOR_ANON_KEY=${anon}
REACTOR_URL=http://${ref}.apps.localhost:18000
EOF
    echo "accept ok"
    exit 0
  fi
  sleep 1
done
docker logs reactor-v2-rie || true
exit 1
