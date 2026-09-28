#!/bin/sh
set -eu
if [ "${1:-}" = "jwt-init" ]; then
  exec /usr/local/bin/reactor-server jwt-init --out "${2:-/keys}"
fi

jwt_dir=/keys
if [ -n "${REACTOR_JWT_PRIVATE_PEM:-}" ] && [ -n "${REACTOR_JWT_PUBLIC_JWK:-}" ]; then
  jwt_dir=/tmp/keys
  mkdir -p "$jwt_dir"
  printf '%s' "$REACTOR_JWT_PRIVATE_PEM" | sed 's/\\n/\n/g' > "$jwt_dir/private.pem"
  printf '%s' "$REACTOR_JWT_PUBLIC_JWK" | sed 's/\\n/\n/g' > "$jwt_dir/public.jwk"
  export REACTOR_AUTH__JWT_PRIVATE_PEM_FILE="$jwt_dir/private.pem"
fi

export PGRST_JWT_SECRET="@${jwt_dir}/public.jwk"
export PGRST_DB_ANON_ROLE=anon
export PGRST_DB_PRE_REQUEST=reactor.pre_request
export PGRST_DB_EXTRA_SEARCH_PATH=
export PGRST_SERVER_HOST=127.0.0.1

if [ -z "${PGRST_DB_URI:-}" ]; then
  PGRST_DB_URI="postgres://authenticator:${AUTH_PASSWORD}@${PGHOST}:5432/reactor"
fi

if [ -n "${REACTOR_PGRST_CONF:-}" ]; then
  mkdir -p "$(dirname "$REACTOR_PGRST_CONF")"
  cat > "$REACTOR_PGRST_CONF" <<EOF
db-uri = "${PGRST_DB_URI}"
db-schemas = "reactor_api"
db-anon-role = "anon"
db-pre-request = "reactor.pre_request"
db-extra-search-path = ""
server-host = "127.0.0.1"
server-port = 3000
jwt-secret = "@${jwt_dir}/public.jwk"
EOF
fi

/usr/local/bin/reactor-server migrate

if [ -n "${REACTOR_PGRST_CONF:-}" ]; then
  /usr/local/bin/postgrest "$REACTOR_PGRST_CONF" &
else
  PGRST_DB_URI="$PGRST_DB_URI" \
    PGRST_SERVER_PORT=3000 \
    /usr/local/bin/postgrest &
fi

if [ -n "${DEDICATED_PG_HOST:-}" ]; then
  PGRST_DB_URI="postgres://authenticator:${AUTH_PASSWORD}@${DEDICATED_PG_HOST}:5432/reactor" \
    PGRST_SERVER_PORT=3001 \
    /usr/local/bin/postgrest &
fi

exec /usr/local/bin/reactor-server
