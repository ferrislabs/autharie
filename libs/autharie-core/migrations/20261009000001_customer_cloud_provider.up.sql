CREATE TABLE cloud_credential_secrets (
    id UUID PRIMARY KEY,
    organisation_id UUID NOT NULL REFERENCES organisations(id),
    provider TEXT NOT NULL,
    ciphertext BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    wrapped_dek TEXT NOT NULL,
    key_provider TEXT NOT NULL,
    key_name TEXT NOT NULL,
    key_version INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT cloud_credential_secrets_provider_known CHECK (provider IN ('scaleway')),
    CONSTRAINT cloud_credential_secrets_nonce_is_96_bits CHECK (octet_length(nonce) = 12)
);

CREATE TABLE cloud_credentials (
    id UUID PRIMARY KEY REFERENCES cloud_credential_secrets(id) ON DELETE CASCADE,
    organisation_id UUID NOT NULL REFERENCES organisations(id),
    provider TEXT NOT NULL,
    label TEXT NOT NULL,
    scope_checked_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT cloud_credentials_provider_known CHECK (provider IN ('scaleway')),
    CONSTRAINT cloud_credentials_label_not_blank CHECK (btrim(label) <> '')
);

CREATE INDEX idx_cloud_credentials_organisation ON cloud_credentials (organisation_id, created_at);

ALTER TABLE deployments
    ADD COLUMN distribution TEXT NOT NULL DEFAULT 'shared',
    ADD COLUMN credential_id UUID NULL REFERENCES cloud_credentials(id) ON DELETE RESTRICT,
    ADD COLUMN cluster_profile JSONB NULL,
    ADD CONSTRAINT deployments_distribution_known
        CHECK (distribution IN ('shared', 'self_hosted', 'customer_cloud')),
    ADD CONSTRAINT deployments_customer_cloud_is_whole
        CHECK (
            (distribution = 'customer_cloud'
                AND credential_id IS NOT NULL
                AND cluster_profile IS NOT NULL)
            OR (distribution <> 'customer_cloud'
                AND credential_id IS NULL
                AND cluster_profile IS NULL)
        );

CREATE INDEX idx_deployments_credential
    ON deployments (credential_id)
    WHERE credential_id IS NOT NULL;

ALTER TABLE data_planes
    ADD COLUMN deployment_id UUID NULL,
    ADD COLUMN credential_id UUID NULL,
    ADD COLUMN failure_reason TEXT NULL,
    DROP CONSTRAINT data_planes_dedicated_has_owner,
    ADD CONSTRAINT data_planes_owner_by_mode
        CHECK (
            (mode = 'dedicated'
                AND organisation_id IS NOT NULL
                AND deployment_id IS NULL
                AND credential_id IS NULL)
            OR (mode = 'customer'
                AND organisation_id IS NOT NULL
                AND deployment_id IS NOT NULL
                AND credential_id IS NOT NULL)
            OR (mode NOT IN ('dedicated', 'customer')
                AND organisation_id IS NULL
                AND deployment_id IS NULL
                AND credential_id IS NULL)
        ),
    ADD CONSTRAINT data_planes_failure_reason_only_when_failed
        CHECK (failure_reason IS NULL OR status = 'failed');

CREATE INDEX idx_data_planes_customer_credential
    ON data_planes (credential_id)
    WHERE mode = 'customer';

CREATE TABLE cluster_inventory (
    data_plane_id UUID NOT NULL,
    kind TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    released_at TIMESTAMPTZ NULL,
    PRIMARY KEY (data_plane_id, kind, provider_id),
    CONSTRAINT cluster_inventory_kind_known
        CHECK (kind IN ('cluster', 'node_pool', 'private_network'))
);

CREATE INDEX idx_cluster_inventory_unreleased
    ON cluster_inventory (data_plane_id)
    WHERE released_at IS NULL;
