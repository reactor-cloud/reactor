CREATE TABLE IF NOT EXISTS reactor.agent_threads (
  id uuid PRIMARY KEY,
  operator_id uuid NOT NULL REFERENCES reactor.operators(id) ON DELETE CASCADE,
  project_ref text,
  running boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS agent_threads_operator ON reactor.agent_threads (operator_id, created_at DESC);

CREATE TABLE IF NOT EXISTS reactor.agent_messages (
  id bigserial PRIMARY KEY,
  thread_id uuid NOT NULL REFERENCES reactor.agent_threads(id) ON DELETE CASCADE,
  seq int NOT NULL,
  role text NOT NULL,
  content text NOT NULL DEFAULT '',
  tool_calls jsonb,
  tool_call_id text,
  name text,
  UNIQUE (thread_id, seq)
);

CREATE INDEX IF NOT EXISTS agent_messages_thread_seq ON reactor.agent_messages (thread_id, seq);
