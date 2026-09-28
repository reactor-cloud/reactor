CREATE TABLE IF NOT EXISTS reactor.function_pins (
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  name text NOT NULL,
  version int NOT NULL,
  PRIMARY KEY (project_id, name)
);
