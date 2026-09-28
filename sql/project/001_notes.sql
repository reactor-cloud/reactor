CREATE TABLE IF NOT EXISTS notes (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id uuid,
  body text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE notes ENABLE ROW LEVEL SECURITY;
ALTER TABLE notes FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS notes_owner ON notes;
CREATE POLICY notes_owner ON notes
  FOR ALL TO authenticated
  USING (user_id::text = current_setting('request.jwt.claims', true)::json->>'sub')
  WITH CHECK (user_id::text = current_setting('request.jwt.claims', true)::json->>'sub');

GRANT SELECT, INSERT, UPDATE, DELETE ON notes TO authenticated, service;

CREATE TABLE IF NOT EXISTS note_pairs (
  id int GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  body text NOT NULL
);

ALTER TABLE note_pairs ENABLE ROW LEVEL SECURITY;
ALTER TABLE note_pairs FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS note_pairs_all ON note_pairs;
CREATE POLICY note_pairs_all ON note_pairs
  FOR ALL TO authenticated
  USING (true)
  WITH CHECK (true);

GRANT SELECT, INSERT ON note_pairs TO authenticated, service;

CREATE OR REPLACE FUNCTION create_note_pair(a text, b text) RETURNS void
LANGUAGE plpgsql
AS $$
BEGIN
  INSERT INTO note_pairs (body) VALUES (a);
  INSERT INTO note_pairs (body) VALUES (b);
END;
$$;

GRANT EXECUTE ON FUNCTION create_note_pair(text, text) TO authenticated, service;
