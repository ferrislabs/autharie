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
CREATE UNIQUE INDEX idx_cells_instance_deployment ON cells (instance_deployment_id);

ALTER TABLE deployments
    ADD COLUMN cell_id UUID NULL REFERENCES cells(id) ON DELETE RESTRICT,
    ADD COLUMN realm TEXT NULL,
    ADD COLUMN cell_slot_held BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN realm_refusal TEXT NULL,
    ADD COLUMN realm_resent_at TIMESTAMPTZ NULL,
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
        ),
    ADD CONSTRAINT deployments_realm_is_hostname_slug
        CHECK (distribution <> 'pooled' OR realm = hostname_slug),
    ADD CONSTRAINT deployments_realm_refusal_is_pooled
        CHECK (realm_refusal IS NULL OR distribution = 'pooled'),
    ADD CONSTRAINT deployments_realm_resent_at_is_pooled
        CHECK (realm_resent_at IS NULL OR distribution = 'pooled');

CREATE UNIQUE INDEX idx_deployments_live_realm
    ON deployments (realm)
    WHERE realm IS NOT NULL AND (deleted_at IS NULL OR cell_slot_held);

CREATE INDEX idx_deployments_cell_slot
    ON deployments (cell_id)
    WHERE cell_slot_held;

CREATE TABLE tenant_administrators (
    deployment_id UUID PRIMARY KEY REFERENCES deployments(id) ON DELETE CASCADE,
    request_id UUID NOT NULL,
    requested_at TIMESTAMPTZ NOT NULL,
    issued_request_id UUID NULL,
    username TEXT NULL,
    ciphertext BYTEA NULL,
    nonce BYTEA NULL,
    wrapped_dek TEXT NULL,
    key_provider TEXT NULL,
    key_name TEXT NULL,
    key_version INTEGER NULL,
    issued_at TIMESTAMPTZ NULL,
    expires_at TIMESTAMPTZ NULL,
    revealed_at TIMESTAMPTZ NULL,
    CONSTRAINT tenant_administrators_sealed_whole CHECK (
        (ciphertext IS NULL AND nonce IS NULL AND wrapped_dek IS NULL
            AND key_provider IS NULL AND key_name IS NULL AND key_version IS NULL)
        OR (ciphertext IS NOT NULL AND nonce IS NOT NULL AND wrapped_dek IS NOT NULL
            AND key_provider IS NOT NULL AND key_name IS NOT NULL AND key_version IS NOT NULL)
    ),
    CONSTRAINT tenant_administrators_issued_whole CHECK (
        (issued_at IS NULL AND expires_at IS NULL AND username IS NULL AND issued_request_id IS NULL)
        OR (issued_at IS NOT NULL AND expires_at IS NOT NULL AND username IS NOT NULL
            AND issued_request_id IS NOT NULL)
    ),
    CONSTRAINT tenant_administrators_sealed_once_issued CHECK (ciphertext IS NULL OR issued_at IS NOT NULL),
    CONSTRAINT tenant_administrators_revealed_once_issued CHECK (revealed_at IS NULL OR issued_at IS NOT NULL),
    CONSTRAINT tenant_administrators_nonce_is_96_bits CHECK (nonce IS NULL OR octet_length(nonce) = 12),
    CONSTRAINT tenant_administrators_revealed_is_wiped CHECK (revealed_at IS NULL OR ciphertext IS NULL)
);

CREATE TABLE platform_cells_settings (
    id              BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (id),
    organisation_id UUID        NULL REFERENCES organisations (id) ON DELETE SET NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by      TEXT
);

INSERT INTO platform_cells_settings (id) VALUES (TRUE);
