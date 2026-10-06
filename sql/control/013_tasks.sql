CREATE TABLE IF NOT EXISTS reactor.tasks (
  id uuid PRIMARY KEY,
  kind text NOT NULL,
  project_id uuid REFERENCES reactor.projects(id) ON DELETE CASCADE,
  payload jsonb NOT NULL DEFAULT '{}',
  run_at timestamptz NOT NULL,
  attempts int NOT NULL DEFAULT 0,
  max_attempts int NOT NULL,
  lease_until timestamptz,
  status text NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'done', 'dead')),
  last_error text,
  dedupe_key text,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS tasks_queued_run_at ON reactor.tasks (run_at) WHERE status = 'queued';

CREATE UNIQUE INDEX IF NOT EXISTS tasks_dedupe_queued
  ON reactor.tasks (kind, dedupe_key)
  WHERE status = 'queued' AND dedupe_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS reactor.ticks (
  name text PRIMARY KEY,
  last_run timestamptz
);
