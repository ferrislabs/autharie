-- A deployment's hostname is flat (`<name>.<zone>`), so its slug must be
-- unique across the whole platform, not per organisation. Same liveness
-- predicate as before: a deleted deployment frees its name.
--
-- Fails if two live deployments of different organisations already share a
-- slug; those must be renamed before this migration is applied.
DROP INDEX IF EXISTS idx_deployments_hostname_slug;

CREATE UNIQUE INDEX idx_deployments_hostname_slug
    ON deployments (hostname_slug)
    WHERE deleted_at IS NULL;
