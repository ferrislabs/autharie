CREATE TABLE cells (
    id UUID PRIMARY KEY,
    region TEXT NOT NULL,
    data_plane_id UUID NOT NULL REFERENCES data_planes(id) ON DELETE RESTRICT,
    instance_deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE RESTRICT,
    status TEXT NOT NULL,
    capacity INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT cells_status_known
        CHECK (status IN ('provisioning', 'open', 'draining', 'retired', 'failed')),
    CONSTRAINT cells_capacity_in_range
        CHECK (capacity > 0 AND capacity <= 65535)
);

CREATE INDEX idx_cells_region_status ON cells (region, status);

ALTER TABLE deployments
    ADD COLUMN cell_id UUID NULL REFERENCES cells(id) ON DELETE RESTRICT,
    ADD COLUMN realm TEXT NULL,
    ADD COLUMN cell_slot_held BOOLEAN NOT NULL DEFAULT false,
    DROP CONSTRAINT deployments_distribution_known,
    ADD CONSTRAINT deployments_distribution_known
        CHECK (distribution IN ('shared', 'self_hosted', 'customer_cloud', 'pooled')),
    ADD CONSTRAINT deployments_pooled_is_whole
        CHECK (
            (distribution = 'pooled'
                AND cell_id IS NOT NULL
                AND realm IS NOT NULL)
            OR (distribution <> 'pooled'
                AND cell_id IS NULL
                AND realm IS NULL
                AND cell_slot_held = false)
        );

CREATE UNIQUE INDEX idx_deployments_live_realm
    ON deployments (realm)
    WHERE deleted_at IS NULL AND realm IS NOT NULL;

CREATE INDEX idx_deployments_cell_slot
    ON deployments (cell_id)
    WHERE cell_slot_held;
