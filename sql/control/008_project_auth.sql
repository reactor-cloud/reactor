ALTER TABLE reactor.users ADD COLUMN IF NOT EXISTS email_verified_at timestamptz;

CREATE TABLE IF NOT EXISTS reactor.auth_settings (
  project_id uuid PRIMARY KEY REFERENCES reactor.projects(id) ON DELETE CASCADE,
  require_email_verification boolean NOT NULL DEFAULT true,
  require_mfa boolean NOT NULL DEFAULT false
);

ALTER TABLE reactor.auth_challenges DROP CONSTRAINT IF EXISTS auth_challenges_kind_check;
ALTER TABLE reactor.auth_challenges ADD CONSTRAINT auth_challenges_kind_check
  CHECK (kind IN ('magic_link', 'recovery', 'invite', 'confirm_email', 'mfa', 'enroll', 'oauth_code', 'otp'));

ALTER TABLE reactor.auth_challenges ADD COLUMN IF NOT EXISTS code_hash text;
ALTER TABLE reactor.auth_challenges ADD COLUMN IF NOT EXISTS attempts int NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS reactor.user_factors (
  user_id uuid NOT NULL REFERENCES reactor.users(id) ON DELETE CASCADE,
  kind text NOT NULL CHECK (kind IN ('totp', 'passkey')),
  secret text,
  credential jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, kind)
);

CREATE TABLE IF NOT EXISTS reactor.user_challenges (
  id uuid PRIMARY KEY,
  user_id uuid NOT NULL REFERENCES reactor.users(id) ON DELETE CASCADE,
  kind text NOT NULL,
  state jsonb NOT NULL,
  expires_at timestamptz NOT NULL
);

CREATE TABLE IF NOT EXISTS reactor.user_recovery_codes (
  user_id uuid NOT NULL REFERENCES reactor.users(id) ON DELETE CASCADE,
  code_hash text NOT NULL,
  used_at timestamptz,
  PRIMARY KEY (user_id, code_hash)
);

CREATE TABLE IF NOT EXISTS reactor.user_identities (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES reactor.users(id) ON DELETE CASCADE,
  provider text NOT NULL,
  subject text NOT NULL,
  email text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (project_id, provider, subject)
);

CREATE TABLE IF NOT EXISTS reactor.oauth_clients (
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  provider text NOT NULL,
  client_id text NOT NULL,
  secret_enc text NOT NULL DEFAULT '',
  extra jsonb NOT NULL DEFAULT '{}',
  enabled boolean NOT NULL DEFAULT true,
  redirects text[] NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, provider)
);

CREATE TABLE IF NOT EXISTS reactor.oauth_transactions (
  state text PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  provider text NOT NULL,
  verifier text NOT NULL,
  redirect_to text NOT NULL,
  expires_at timestamptz NOT NULL
);
