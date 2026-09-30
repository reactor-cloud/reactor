CREATE TABLE IF NOT EXISTS reactor.cluster_email (
  id int PRIMARY KEY DEFAULT 1 CHECK (id = 1),
  host text NOT NULL DEFAULT '',
  port int NOT NULL DEFAULT 587,
  username text NOT NULL DEFAULT '',
  password_enc text NOT NULL DEFAULT '',
  from_address text NOT NULL DEFAULT '',
  tls text NOT NULL DEFAULT 'starttls' CHECK (tls IN ('starttls', 'tls', 'none'))
);

INSERT INTO reactor.cluster_email (id) VALUES (1) ON CONFLICT (id) DO NOTHING;

ALTER TABLE reactor.projects ADD COLUMN IF NOT EXISTS cluster_smtp boolean NOT NULL DEFAULT false;
