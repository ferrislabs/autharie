DROP INDEX IF EXISTS idx_deployments_hostname_slug;

CREATE UNIQUE INDEX idx_deployments_hostname_slug
    ON deployments (organisation_id, hostname_slug)
    WHERE deleted_at IS NULL;
