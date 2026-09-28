CREATE TABLE IF NOT EXISTS reactor.settings (
  id int PRIMARY KEY DEFAULT 1 CHECK (id = 1),
  cluster_name text NOT NULL,
  setup_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS reactor.operators (
  id uuid PRIMARY KEY,
  email text NOT NULL UNIQUE,
  name text NOT NULL DEFAULT '',
  password_hash text NOT NULL,
  platform_admin boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS reactor.memberships (
  operator_id uuid NOT NULL REFERENCES reactor.operators(id) ON DELETE CASCADE,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  role text NOT NULL CHECK (role IN ('owner', 'admin', 'developer')),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (operator_id, project_id)
);

CREATE TABLE IF NOT EXISTS reactor.logs (
  id bigserial PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  kind text NOT NULL,
  name text NOT NULL,
  status int NOT NULL,
  message text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS logs_project_created ON reactor.logs (project_id, created_at DESC);
