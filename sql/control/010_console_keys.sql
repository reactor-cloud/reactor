CREATE TABLE IF NOT EXISTS reactor.console_keys (
  id uuid PRIMARY KEY,
  operator_id uuid NOT NULL REFERENCES reactor.operators(id) ON DELETE CASCADE,
  name text NOT NULL,
  token_hash text NOT NULL UNIQUE,
  scopes text[] NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE reactor.projects
  ADD COLUMN IF NOT EXISTS created_by_key uuid REFERENCES reactor.console_keys(id) ON DELETE SET NULL;
