CREATE TABLE IF NOT EXISTS reactor.mcp_clients (
  id text PRIMARY KEY,
  name text NOT NULL DEFAULT '',
  redirect_uris text[] NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS reactor.oauth_codes (
  code_hash text PRIMARY KEY,
  client_id text NOT NULL,
  operator_id uuid NOT NULL REFERENCES reactor.operators(id) ON DELETE CASCADE,
  redirect_uri text NOT NULL,
  code_challenge text NOT NULL,
  scope text NOT NULL,
  resource text NOT NULL,
  expires_at timestamptz NOT NULL,
  consumed_at timestamptz
);

CREATE TABLE IF NOT EXISTS reactor.oauth_refresh (
  id uuid PRIMARY KEY,
  family_id uuid NOT NULL,
  token_hash text NOT NULL UNIQUE,
  client_id text NOT NULL,
  operator_id uuid NOT NULL REFERENCES reactor.operators(id) ON DELETE CASCADE,
  scope text NOT NULL,
  resource text NOT NULL,
  expires_at timestamptz NOT NULL,
  rotated_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS oauth_refresh_family ON reactor.oauth_refresh (family_id);

CREATE TABLE IF NOT EXISTS reactor.mcp_keys (
  id uuid PRIMARY KEY,
  operator_id uuid NOT NULL REFERENCES reactor.operators(id) ON DELETE CASCADE,
  name text NOT NULL,
  token_hash text NOT NULL UNIQUE,
  scope text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
