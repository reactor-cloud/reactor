-- Control plane. Idempotent. Authenticator password is replaced by the server.

CREATE SCHEMA IF NOT EXISTS reactor;
CREATE SCHEMA IF NOT EXISTS reactor_api;

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'anon') THEN
    CREATE ROLE anon NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticated') THEN
    CREATE ROLE authenticated NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'service') THEN
    CREATE ROLE service NOLOGIN BYPASSRLS;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticator') THEN
    CREATE ROLE authenticator NOINHERIT LOGIN PASSWORD '__AUTH_PASSWORD__';
  END IF;
END
$$;

ALTER ROLE service BYPASSRLS;
ALTER ROLE authenticator NOINHERIT;
ALTER ROLE authenticator PASSWORD '__AUTH_PASSWORD__';
ALTER ROLE authenticator SET search_path TO '';
GRANT anon, authenticated, service TO authenticator;
GRANT USAGE ON SCHEMA reactor TO authenticator, anon, authenticated, service;
GRANT USAGE ON SCHEMA reactor_api TO authenticator, anon, authenticated, service;

CREATE TABLE IF NOT EXISTS reactor.projects (
  id uuid PRIMARY KEY,
  ref text NOT NULL UNIQUE CHECK (ref ~ '^[a-z0-9]{20}$'),
  name text NOT NULL DEFAULT '',
  database_url text,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS reactor.schema_migrations (
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  version text NOT NULL,
  applied_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, version)
);

CREATE TABLE IF NOT EXISTS reactor.users (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  email text NOT NULL,
  password_hash text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (project_id, email)
);

CREATE TABLE IF NOT EXISTS reactor.sessions (
  id uuid PRIMARY KEY,
  user_id uuid NOT NULL REFERENCES reactor.users(id) ON DELETE CASCADE,
  token_hash text NOT NULL UNIQUE,
  expires_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS reactor.api_keys (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  role text NOT NULL,
  token_hash text NOT NULL UNIQUE,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS reactor.deployments (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  name text NOT NULL,
  version int NOT NULL,
  blob_key text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (project_id, name, version)
);

CREATE TABLE IF NOT EXISTS reactor.schedules (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  function_name text NOT NULL,
  body text NOT NULL DEFAULT '{}',
  next_run timestamptz NOT NULL
);

CREATE TABLE IF NOT EXISTS reactor.site_files (
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  path text NOT NULL,
  blob_key text NOT NULL,
  content_type text NOT NULL,
  PRIMARY KEY (project_id, path)
);

CREATE TABLE IF NOT EXISTS reactor.site_routes (
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  path text NOT NULL,
  function_name text NOT NULL,
  PRIMARY KEY (project_id, path)
);

CREATE TABLE IF NOT EXISTS reactor.domains (
  host text PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES reactor.projects(id) ON DELETE CASCADE,
  token text NOT NULL,
  verified_at timestamptz
);

CREATE OR REPLACE FUNCTION reactor.pre_request() RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = reactor, pg_temp
AS $$
DECLARE
  claims json;
  ref text;
BEGIN
  claims := current_setting('request.jwt.claims', true)::json;
  ref := claims->>'ref';
  IF ref IS NULL OR ref !~ '^[a-z0-9]{20}$' THEN
    RAISE EXCEPTION 'invalid project ref' USING ERRCODE = '28000';
  END IF;
  PERFORM set_config('search_path', 'proj_' || ref, true);
END;
$$;

REVOKE ALL ON FUNCTION reactor.pre_request() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION reactor.pre_request() TO authenticator, anon, authenticated, service;

DO $guard$
BEGIN
  EXECUTE 'ALTER ROLE authenticator SET pgrst.db_schemas = ''reactor_api''';
  EXECUTE 'ALTER ROLE authenticator SET pgrst.db_pre_request = ''reactor.pre_request''';
EXCEPTION
  WHEN insufficient_privilege THEN
    NULL;
END
$guard$;
