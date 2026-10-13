ALTER TABLE deployments
    ADD COLUMN realm_refusal TEXT NULL,
    ADD CONSTRAINT deployments_realm_refusal_is_pooled
        CHECK (realm_refusal IS NULL OR distribution = 'pooled');
