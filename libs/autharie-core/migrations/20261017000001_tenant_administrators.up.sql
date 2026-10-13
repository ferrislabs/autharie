CREATE TABLE tenant_administrators (
    deployment_id UUID PRIMARY KEY REFERENCES deployments(id) ON DELETE CASCADE,
    requested_at TIMESTAMPTZ NOT NULL,
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
        (issued_at IS NULL AND expires_at IS NULL AND username IS NULL)
        OR (issued_at IS NOT NULL AND expires_at IS NOT NULL AND username IS NOT NULL)
    ),
    CONSTRAINT tenant_administrators_sealed_once_issued CHECK (ciphertext IS NULL OR issued_at IS NOT NULL),
    CONSTRAINT tenant_administrators_revealed_once_issued CHECK (revealed_at IS NULL OR issued_at IS NOT NULL),
    CONSTRAINT tenant_administrators_nonce_is_96_bits CHECK (nonce IS NULL OR octet_length(nonce) = 12),
    CONSTRAINT tenant_administrators_revealed_is_wiped CHECK (revealed_at IS NULL OR ciphertext IS NULL)
);
