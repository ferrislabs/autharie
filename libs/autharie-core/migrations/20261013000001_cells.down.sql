DROP INDEX idx_deployments_cell_slot;
DROP INDEX idx_deployments_live_realm;

DELETE FROM deployments WHERE distribution = 'pooled';

ALTER TABLE deployments
    DROP CONSTRAINT deployments_pooled_is_whole,
    DROP CONSTRAINT deployments_distribution_known,
    ADD CONSTRAINT deployments_distribution_known
        CHECK (distribution IN ('shared', 'self_hosted', 'customer_cloud')),
    DROP COLUMN cell_slot_held,
    DROP COLUMN realm,
    DROP COLUMN cell_id;

DROP TABLE cells;
