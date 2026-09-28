CREATE TABLE IF NOT EXISTS reactor.function_env (
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  name text NOT NULL,
  key text NOT NULL,
  nonce bytea NOT NULL,
  ciphertext bytea NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, name, key)
);
