ALTER TABLE deployments
    DROP CONSTRAINT deployments_realm_refusal_is_pooled,
    DROP COLUMN realm_refusal;
