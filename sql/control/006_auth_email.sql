ALTER TABLE reactor.users ALTER COLUMN password_hash DROP NOT NULL;

CREATE TABLE IF NOT EXISTS reactor.email_settings (
  project_id uuid PRIMARY KEY REFERENCES reactor.projects(id) ON DELETE CASCADE,
  host text NOT NULL,
  port int NOT NULL,
  username text NOT NULL DEFAULT '',
  password_enc text NOT NULL DEFAULT '',
  from_address text NOT NULL,
  tls text NOT NULL DEFAULT 'starttls' CHECK (tls IN ('starttls', 'tls', 'none')),
  link_base text NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS reactor.email_templates (
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  name text NOT NULL CHECK (name ~ '^[a-z][a-z0-9_]{0,63}$'),
  subject text NOT NULL,
  body_html text NOT NULL DEFAULT '',
  body_text text NOT NULL DEFAULT '',
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, name)
);

CREATE TABLE IF NOT EXISTS reactor.auth_challenges (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  user_id uuid REFERENCES reactor.users(id) ON DELETE CASCADE,
  email text NOT NULL,
  kind text NOT NULL CHECK (kind IN ('magic_link', 'recovery', 'invite')),
  token_hash text NOT NULL UNIQUE,
  expires_at timestamptz NOT NULL,
  consumed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS auth_challenges_email ON reactor.auth_challenges (project_id, email, kind, created_at DESC);

CREATE TABLE IF NOT EXISTS reactor.operator_factors (
  operator_id uuid NOT NULL REFERENCES reactor.operators(id) ON DELETE CASCADE,
  kind text NOT NULL CHECK (kind IN ('totp', 'passkey')),
  secret text,
  credential jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (operator_id, kind)
);

CREATE TABLE IF NOT EXISTS reactor.operator_challenges (
  id uuid PRIMARY KEY,
  operator_id uuid NOT NULL REFERENCES reactor.operators(id) ON DELETE CASCADE,
  kind text NOT NULL,
  state jsonb NOT NULL,
  expires_at timestamptz NOT NULL
);

CREATE TABLE IF NOT EXISTS reactor.operator_recovery_codes (
  operator_id uuid NOT NULL REFERENCES reactor.operators(id) ON DELETE CASCADE,
  code_hash text NOT NULL,
  used_at timestamptz,
  PRIMARY KEY (operator_id, code_hash)
);
