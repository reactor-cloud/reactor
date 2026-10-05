CREATE INDEX IF NOT EXISTS auth_challenges_kind_recent
  ON reactor.auth_challenges (project_id, kind, created_at DESC);
