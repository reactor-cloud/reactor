ALTER TABLE reactor.agent_threads ADD COLUMN IF NOT EXISTS model text;

ALTER TABLE reactor.agent_messages ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now();
