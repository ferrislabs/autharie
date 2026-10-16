ALTER TABLE deployments
    DROP CONSTRAINT deployments_realm_resent_at_is_pooled,
    DROP COLUMN realm_resent_at;
