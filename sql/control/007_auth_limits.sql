CREATE TABLE IF NOT EXISTS reactor.auth_attempts (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  ip text NOT NULL,
  kind text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS auth_attempts_lookup
  ON reactor.auth_attempts (project_id, ip, kind, created_at DESC);
