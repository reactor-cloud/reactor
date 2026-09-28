CREATE TABLE IF NOT EXISTS reactor.site_deployments (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  status text NOT NULL,
  error text NOT NULL DEFAULT '',
  file_count int NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS reactor.site_deployment_files (
  deployment_id uuid NOT NULL REFERENCES reactor.site_deployments(id) ON DELETE CASCADE,
  path text NOT NULL,
  blob_key text NOT NULL,
  content_type text NOT NULL,
  PRIMARY KEY (deployment_id, path)
);

CREATE TABLE IF NOT EXISTS reactor.site_env (
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  key text NOT NULL,
  visibility text NOT NULL,
  nonce bytea NOT NULL,
  ciphertext bytea NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, key)
);
