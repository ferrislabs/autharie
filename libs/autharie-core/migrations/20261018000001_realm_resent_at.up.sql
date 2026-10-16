ALTER TABLE deployments
    ADD COLUMN realm_resent_at TIMESTAMPTZ NULL,
    ADD CONSTRAINT deployments_realm_resent_at_is_pooled
        CHECK (realm_resent_at IS NULL OR distribution = 'pooled');
