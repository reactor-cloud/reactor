CREATE TABLE IF NOT EXISTS storage_buckets (
  id text PRIMARY KEY,
  "public" boolean NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS storage_objects (
  bucket_id text NOT NULL REFERENCES storage_buckets(id) ON DELETE CASCADE,
  name text NOT NULL,
  owner uuid,
  PRIMARY KEY (bucket_id, name)
);

ALTER TABLE storage_buckets ENABLE ROW LEVEL SECURITY;
ALTER TABLE storage_buckets FORCE ROW LEVEL SECURITY;
ALTER TABLE storage_objects ENABLE ROW LEVEL SECURITY;
ALTER TABLE storage_objects FORCE ROW LEVEL SECURITY;

GRANT SELECT ON storage_objects TO anon;
GRANT SELECT, INSERT, UPDATE, DELETE ON storage_buckets TO authenticated, service;
GRANT SELECT, INSERT, UPDATE, DELETE ON storage_objects TO authenticated, service;
