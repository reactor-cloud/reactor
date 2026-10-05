CREATE TABLE IF NOT EXISTS reactor.project_db_roles (
  project_id uuid PRIMARY KEY REFERENCES reactor.projects(id) ON DELETE CASCADE,
  password text NOT NULL
);

ALTER TABLE reactor.schema_migrations ADD COLUMN IF NOT EXISTS sql text;
ALTER TABLE reactor.schema_migrations ADD COLUMN IF NOT EXISTS down_sql text;
ALTER TABLE reactor.schema_migrations ADD COLUMN IF NOT EXISTS applied_by uuid;
ALTER TABLE reactor.schema_migrations ADD COLUMN IF NOT EXISTS source text;
ALTER TABLE reactor.schema_migrations ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT 'applied';
