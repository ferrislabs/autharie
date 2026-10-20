DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM deployments WHERE distribution = 'pooled') THEN
        RAISE EXCEPTION 'pooled deployments exist: delete them before dropping the cells';
    END IF;
END $$;

DROP TABLE platform_cells_settings;
DROP TABLE tenant_administrators;

DROP INDEX idx_deployments_cell_slot;
DROP INDEX idx_deployments_live_realm;

ALTER TABLE deployments
    DROP CONSTRAINT deployments_realm_resent_at_is_pooled,
    DROP CONSTRAINT deployments_realm_refusal_is_pooled,
    DROP CONSTRAINT deployments_realm_is_hostname_slug,
    DROP CONSTRAINT deployments_pooled_is_whole,
    DROP CONSTRAINT deployments_distribution_known,
    ADD CONSTRAINT deployments_distribution_known
        CHECK (distribution IN ('shared', 'self_hosted', 'customer_cloud')),
    DROP COLUMN realm_resent_at,
    DROP COLUMN realm_refusal,
    DROP COLUMN cell_slot_held,
    DROP COLUMN realm,
    DROP COLUMN cell_id;

DROP TABLE cells;
