DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM deployments WHERE distribution = 'pooled') THEN
        RAISE EXCEPTION 'pooled deployments exist: delete them before reverting the cells migration';
    END IF;
END $$;

DROP INDEX idx_deployments_cell_slot;
DROP INDEX idx_deployments_live_realm;

ALTER TABLE deployments
    DROP CONSTRAINT deployments_realm_is_hostname_slug,
    DROP CONSTRAINT deployments_pooled_is_whole,
    DROP CONSTRAINT deployments_distribution_known,
    ADD CONSTRAINT deployments_distribution_known
        CHECK (distribution IN ('shared', 'self_hosted', 'customer_cloud')),
    DROP COLUMN cell_slot_held,
    DROP COLUMN realm,
    DROP COLUMN cell_id;

DROP TABLE cells;
