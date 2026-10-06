CREATE TABLE IF NOT EXISTS queues (
  name text PRIMARY KEY CHECK (name ~ '^[a-z][a-z0-9_]{0,62}$'),
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS queue_messages (
  msg_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  queue_name text NOT NULL REFERENCES queues(name) ON DELETE CASCADE,
  message jsonb NOT NULL,
  vt timestamptz NOT NULL DEFAULT now(),
  read_ct int NOT NULL DEFAULT 0,
  enqueued_at timestamptz NOT NULL DEFAULT now(),
  archived_at timestamptz
);

CREATE INDEX IF NOT EXISTS queue_messages_ready
  ON queue_messages (queue_name, msg_id)
  WHERE archived_at IS NULL;

CREATE TABLE IF NOT EXISTS queue_subscriptions (
  queue_name text NOT NULL REFERENCES queues(name) ON DELETE CASCADE,
  function_name text NOT NULL,
  vt_secs int NOT NULL,
  qty int NOT NULL,
  max_reads int NOT NULL,
  PRIMARY KEY (queue_name, function_name)
);

ALTER TABLE queues ENABLE ROW LEVEL SECURITY;
ALTER TABLE queues FORCE ROW LEVEL SECURITY;
ALTER TABLE queue_messages ENABLE ROW LEVEL SECURITY;
ALTER TABLE queue_messages FORCE ROW LEVEL SECURITY;
ALTER TABLE queue_subscriptions ENABLE ROW LEVEL SECURITY;
ALTER TABLE queue_subscriptions FORCE ROW LEVEL SECURITY;

REVOKE ALL ON queues, queue_messages, queue_subscriptions FROM PUBLIC, anon, authenticated;
GRANT SELECT, INSERT, UPDATE, DELETE ON queues, queue_messages, queue_subscriptions TO service;
REVOKE ALL ON SEQUENCE queue_messages_msg_id_seq FROM PUBLIC, anon, authenticated;
GRANT USAGE, SELECT ON SEQUENCE queue_messages_msg_id_seq TO service;

CREATE OR REPLACE FUNCTION queue_create(qname text) RETURNS void
LANGUAGE plpgsql
SECURITY INVOKER
AS $fn$
BEGIN
  IF qname !~ '^[a-z][a-z0-9_]{0,62}$' THEN
    RAISE EXCEPTION 'invalid queue name';
  END IF;
  INSERT INTO queues (name) VALUES (qname);
END;
$fn$;

CREATE OR REPLACE FUNCTION queue_send(qname text, payload jsonb, delay_secs int) RETURNS bigint
LANGUAGE plpgsql
SECURITY INVOKER
AS $fn$
DECLARE
  id bigint;
BEGIN
  IF NOT EXISTS (SELECT 1 FROM queues WHERE name = qname) THEN
    RAISE EXCEPTION 'queue not found' USING ERRCODE = 'P0002';
  END IF;
  INSERT INTO queue_messages (queue_name, message, vt)
  VALUES (qname, payload, now() + make_interval(secs => GREATEST(delay_secs, 0)))
  RETURNING msg_id INTO id;
  RETURN id;
END;
$fn$;

CREATE OR REPLACE FUNCTION queue_read(qname text, vt_secs int, qty int)
RETURNS TABLE (msg_id bigint, message jsonb, read_ct int)
LANGUAGE plpgsql
SECURITY INVOKER
AS $fn$
#variable_conflict use_column
BEGIN
  IF qty < 1 THEN
    qty := 1;
  END IF;
  RETURN QUERY
  UPDATE queue_messages AS m
  SET vt = now() + make_interval(secs => GREATEST(vt_secs, 0)),
      read_ct = m.read_ct + 1
  WHERE m.msg_id IN (
    SELECT qm.msg_id
    FROM queue_messages AS qm
    WHERE qm.queue_name = qname
      AND qm.archived_at IS NULL
      AND qm.vt <= now()
    ORDER BY qm.msg_id
    FOR UPDATE SKIP LOCKED
    LIMIT qty
  )
  RETURNING m.msg_id, m.message, m.read_ct;
END;
$fn$;

CREATE OR REPLACE FUNCTION queue_delete(id bigint) RETURNS void
LANGUAGE sql
SECURITY INVOKER
AS $fn$
  DELETE FROM queue_messages WHERE msg_id = id;
$fn$;

CREATE OR REPLACE FUNCTION queue_archive(id bigint) RETURNS void
LANGUAGE sql
SECURITY INVOKER
AS $fn$
  UPDATE queue_messages SET archived_at = now() WHERE msg_id = id AND archived_at IS NULL;
$fn$;

CREATE OR REPLACE FUNCTION queue_peek(qname text)
RETURNS TABLE (msg_id bigint, message jsonb, vt timestamptz, read_ct int, enqueued_at timestamptz)
LANGUAGE sql
SECURITY INVOKER
AS $fn$
  SELECT msg_id, message, vt, read_ct, enqueued_at
  FROM queue_messages
  WHERE queue_name = qname AND archived_at IS NULL
  ORDER BY msg_id
  LIMIT 100;
$fn$;

REVOKE ALL ON FUNCTION queue_create(text) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION queue_send(text, jsonb, int) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION queue_read(text, int, int) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION queue_delete(bigint) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION queue_archive(bigint) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION queue_peek(text) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION queue_create(text) TO service;
GRANT EXECUTE ON FUNCTION queue_send(text, jsonb, int) TO service;
GRANT EXECUTE ON FUNCTION queue_read(text, int, int) TO service;
GRANT EXECUTE ON FUNCTION queue_delete(bigint) TO service;
GRANT EXECUTE ON FUNCTION queue_archive(bigint) TO service;
GRANT EXECUTE ON FUNCTION queue_peek(text) TO service;
