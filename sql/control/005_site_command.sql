ALTER TABLE reactor.site_deployments ADD COLUMN IF NOT EXISTS command text NOT NULL DEFAULT '';
