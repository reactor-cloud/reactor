#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

ORG=reactor-890
REGION=fra
APP=reactor-v2
DB=reactor-v2-db
BACKUP=reactor-v2-backup
STATE="$(cd "$(dirname "$0")" && pwd)/.state"

if [[ -z "${REACTOR_AGENT__API_KEY:-}" ]]; then
  echo "REACTOR_AGENT__API_KEY is required" >&2
  exit 1
fi

mkdir -p "$STATE"
chmod 700 "$STATE"

app_exists() {
  fly apps list --json | python3 -c '
import json, sys
name = sys.argv[1]
apps = json.load(sys.stdin)
def label(app):
    return app.get("Name") or app.get("name") or ""
sys.exit(0 if any(label(app) == name for app in apps) else 1)
' "$1"
}

ensure_app() {
  if app_exists "$1"; then
    echo "app $1 exists"
  else
    fly apps create "$1" --org "$ORG" --yes
  fi
}

secret_exists() {
  fly secrets list -a "$1" | awk 'NR > 1 { print $1 }' | grep -qx "$2"
}

if app_exists "$DB" && secret_exists "$DB" POSTGRES_PASSWORD && [[ ! -f "$STATE/cluster.env" ]]; then
  echo "postgres secret exists but $STATE/cluster.env is missing; refusing to rotate" >&2
  exit 1
fi

if [[ ! -f "$STATE/cluster.env" ]]; then
  db_password="$(openssl rand -hex 24)"
  auth_password="$(openssl rand -hex 24)"
  operator_token="$(openssl rand -hex 32)"
  umask 077
  cat >"$STATE/cluster.env" <<EOF
DB_PASSWORD=$db_password
AUTH_PASSWORD=$auth_password
OPERATOR_TOKEN=$operator_token
EOF
fi
# shellcheck disable=SC1091
source "$STATE/cluster.env"

if [[ ! -f "$STATE/keys/private.pem" ]]; then
  if [[ ! -x target/debug/reactor-server ]]; then
    cargo build -p reactor-server
  fi
  ./target/debug/reactor-server jwt-init --out "$STATE/keys"
fi

db_url="postgres://reactor:${DB_PASSWORD}@${DB}.internal:5432/reactor?sslmode=disable"
auth_url="postgres://authenticator:${AUTH_PASSWORD}@${DB}.internal:5432/reactor?sslmode=disable"

ensure_app "$DB"
ensure_app "$APP"
ensure_app "$BACKUP"

if ! secret_exists "$DB" POSTGRES_PASSWORD; then
  fly secrets set --stage -a "$DB" "POSTGRES_PASSWORD=${DB_PASSWORD}"
fi

volume_id="$(fly volumes list -a "$DB" --json | python3 -c '
import json, sys
vols = json.load(sys.stdin)
named = [v for v in vols if (v.get("name") or v.get("Name")) == "pg_data"]
print((named or vols or [{}])[0].get("id") or (named or vols or [{}])[0].get("ID") or "")
')"
if [[ -z "$volume_id" ]]; then
  fly volumes create pg_data -a "$DB" -r "$REGION" -s 10 --yes
  volume_id="$(fly volumes list -a "$DB" --json | python3 -c '
import json, sys
vols = json.load(sys.stdin)
print(vols[0].get("id") or vols[0].get("ID"))
')"
fi

db_started="$(fly machine list -a "$DB" --json | python3 -c '
import json, sys
machines = json.load(sys.stdin)
sys.exit(0 if any((m.get("state") or m.get("State")) == "started" for m in machines) else 1)
' && echo yes || echo no)"
if [[ "$db_started" != yes ]]; then
  fly deploy --config deploy/fly/postgres/fly.toml --ha=false --image postgres:16 -a "$DB"
fi

ready=0
for _ in $(seq 1 40); do
  if fly checks list -a "$DB" | grep -q passing; then
    ready=1
    break
  fi
  sleep 5
done
if [[ "$ready" != 1 ]]; then
  fly logs -a "$DB" --no-tail || true
  echo "postgres did not pass its tcp check" >&2
  exit 1
fi

snapshot_count="$(fly volumes snapshots list "$volume_id" -a "$DB" --json | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
if [[ "$snapshot_count" == 0 ]]; then
  fly volumes snapshots create "$volume_id" -a "$DB"
fi

if [[ ! -f "$STATE/storage.env" ]]; then
  fly storage create --name reactor-v2 --org "$ORG" --yes | tee "$STATE/storage.create.txt"
  python3 - "$STATE/storage.create.txt" "$STATE/storage.env" <<'PY'
import pathlib, re, sys
text = pathlib.Path(sys.argv[1]).read_text()
text = re.sub(r"\x1b\[[0-9;]*m", "", text)
found = {}
for line in text.splitlines():
    match = re.match(r"\s*(AWS_ACCESS_KEY_ID|AWS_SECRET_ACCESS_KEY|BUCKET_NAME|AWS_ENDPOINT_URL_S3)\s*[:=]\s*(\S+)", line)
    if match:
        found[match.group(1)] = match.group(2)
missing = [key for key in ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "BUCKET_NAME") if key not in found]
if missing:
    sys.stderr.write("storage create did not print %s\n" % ", ".join(missing))
    sys.exit(1)
path = pathlib.Path(sys.argv[2])
path.write_text(
    "BUCKET_NAME=%s\nAWS_ACCESS_KEY_ID=%s\nAWS_SECRET_ACCESS_KEY=%s\n" % (
        found["BUCKET_NAME"], found["AWS_ACCESS_KEY_ID"], found["AWS_SECRET_ACCESS_KEY"]
    )
)
path.chmod(0o600)
PY
fi
# shellcheck disable=SC1091
source "$STATE/storage.env"

if ! secret_exists "$APP" REACTOR_DATABASE__URL; then
  STATE="$STATE" \
  DB_URL="$db_url" \
  AUTH_URL="$auth_url" \
  AUTH_PASSWORD="$AUTH_PASSWORD" \
  OPERATOR_TOKEN="$OPERATOR_TOKEN" \
  BUCKET_NAME="$BUCKET_NAME" \
  AWS_ACCESS_KEY_ID="$AWS_ACCESS_KEY_ID" \
  AWS_SECRET_ACCESS_KEY="$AWS_SECRET_ACCESS_KEY" \
  python3 - <<'PY'
import os
import pathlib
state = pathlib.Path(os.environ["STATE"])
pem = (state / "keys" / "private.pem").read_text().replace("\n", "\\n")
jwk = (state / "keys" / "public.jwk").read_text().strip()
pairs = [
    ("REACTOR_DATABASE__URL", os.environ["DB_URL"]),
    ("REACTOR_DATABASE__AUTHENTICATOR_PASSWORD", os.environ["AUTH_PASSWORD"]),
    ("PGRST_DB_URI", os.environ["AUTH_URL"]),
    ("REACTOR_AUTH__OPERATOR_TOKEN", os.environ["OPERATOR_TOKEN"]),
    ("REACTOR_STORAGE__BUCKET", os.environ["BUCKET_NAME"]),
    ("REACTOR_STORAGE__ACCESS_KEY", os.environ["AWS_ACCESS_KEY_ID"]),
    ("REACTOR_STORAGE__SECRET_KEY", os.environ["AWS_SECRET_ACCESS_KEY"]),
    ("REACTOR_AGENT__API_KEY", os.environ["REACTOR_AGENT__API_KEY"]),
    ("REACTOR_JWT_PRIVATE_PEM", pem),
    ("REACTOR_JWT_PUBLIC_JWK", jwk),
]
path = state / "app.secrets"
path.write_text("".join("%s=%s\n" % item for item in pairs))
path.chmod(0o600)
PY
  fly secrets import --stage -a "$APP" <"$STATE/app.secrets"
fi

if ! secret_exists "$BACKUP" DATABASE_URL; then
  fly secrets set --stage -a "$BACKUP" \
    "DATABASE_URL=${db_url}" \
    "BUCKET_NAME=${BUCKET_NAME}" \
    "AWS_ACCESS_KEY_ID=${AWS_ACCESS_KEY_ID}" \
    "AWS_SECRET_ACCESS_KEY=${AWS_SECRET_ACCESS_KEY}" \
    "AWS_ENDPOINT_URL_S3=https://t3.storage.dev" \
    "AWS_DEFAULT_REGION=auto"
fi

backup_exists="$(fly machine list -a "$BACKUP" --json | python3 -c '
import json, sys
machines = json.load(sys.stdin)
sys.exit(0 if any((m.get("name") or m.get("Name")) == "daily-dump" for m in machines) else 1)
' && echo yes || echo no)"
if [[ "$backup_exists" != yes ]]; then
  (
    cd deploy/fly/backup
    fly machine run . \
      --dockerfile Dockerfile \
      --app "$BACKUP" \
      --name daily-dump \
      --region "$REGION" \
      --schedule daily \
      --restart on-failure \
      --vm-size shared-cpu-1x \
      --vm-memory 512
  )
fi

echo "infra ready"
echo "postgres $volume_id"
echo "next: fly deploy . --config deploy/fly/fly.toml --ha=false"
