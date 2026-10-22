CREATE TABLE deployment_webhook_sync (
    deployment_id UUID PRIMARY KEY REFERENCES deployments(id) ON DELETE CASCADE,
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    applied_revision BIGINT NOT NULL DEFAULT 0 CHECK (applied_revision >= 0),
    realms TEXT[] NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE deployment_webhooks (
    id UUID PRIMARY KEY,
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    realm TEXT NOT NULL,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    description TEXT NULL CHECK (description IS NULL OR char_length(description) <= 500),
    endpoint TEXT NOT NULL CHECK (char_length(endpoint) <= 2048),
    events TEXT[] NOT NULL CHECK (cardinality(events) BETWEEN 1 AND 50),
    header_names TEXT[] NOT NULL DEFAULT '{}' CHECK (cardinality(header_names) <= 10),
    headers_ciphertext BYTEA NULL,
    headers_nonce BYTEA NULL,
    headers_wrapped_dek TEXT NULL,
    headers_key_provider TEXT NULL,
    headers_key_name TEXT NULL,
    headers_key_version INTEGER NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'active', 'failed')),
    ferriskey_id TEXT NULL,
    failure_reason TEXT NULL,
    desired_revision BIGINT NOT NULL CHECK (desired_revision >= 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT deployment_webhooks_headers_sealed_whole CHECK (
        (headers_ciphertext IS NULL) = (headers_nonce IS NULL)
        AND (headers_ciphertext IS NULL) = (headers_wrapped_dek IS NULL)
        AND (headers_ciphertext IS NULL) = (headers_key_provider IS NULL)
        AND (headers_ciphertext IS NULL) = (headers_key_name IS NULL)
        AND (headers_ciphertext IS NULL) = (headers_key_version IS NULL)
    )
);

CREATE INDEX deployment_webhooks_by_deployment ON deployment_webhooks (deployment_id, created_at, id);

CREATE TABLE deployment_webhook_secrets (
    webhook_id UUID PRIMARY KEY REFERENCES deployment_webhooks(id) ON DELETE CASCADE,
    version BIGINT NOT NULL CHECK (version >= 1),
    ciphertext BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    wrapped_dek TEXT NOT NULL,
    key_provider TEXT NOT NULL,
    key_name TEXT NOT NULL,
    key_version INTEGER NOT NULL,
    received_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE deployment_webhook_requests (
    id UUID PRIMARY KEY,
    webhook_id UUID NOT NULL REFERENCES deployment_webhooks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('rotate_secret', 'retry_delivery')),
    delivery_id TEXT NULL,
    phase TEXT NOT NULL DEFAULT 'pending' CHECK (phase IN ('pending', 'done', 'failed')),
    message TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX deployment_webhook_requests_by_webhook ON deployment_webhook_requests (webhook_id, created_at DESC);
