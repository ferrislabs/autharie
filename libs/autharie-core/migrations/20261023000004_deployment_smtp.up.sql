CREATE TABLE deployment_smtp_sync (
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    realm TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (deployment_id, realm)
);

CREATE TABLE deployment_smtp_configs (
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    realm TEXT NOT NULL,
    host TEXT NOT NULL CHECK (char_length(host) BETWEEN 1 AND 253),
    port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    encryption TEXT NOT NULL CHECK (encryption IN ('tls', 'starttls', 'none')),
    username TEXT NOT NULL CHECK (char_length(username) BETWEEN 1 AND 255),
    from_email TEXT NOT NULL CHECK (char_length(from_email) BETWEEN 3 AND 254),
    from_name TEXT NOT NULL CHECK (char_length(from_name) <= 255),
    password_ciphertext BYTEA NOT NULL,
    password_nonce BYTEA NOT NULL,
    password_wrapped_dek TEXT NOT NULL,
    password_key_provider TEXT NOT NULL,
    password_key_name TEXT NOT NULL,
    password_key_version INTEGER NOT NULL,
    phase TEXT NOT NULL DEFAULT 'pending' CHECK (phase IN ('pending', 'applied', 'failed')),
    message TEXT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (deployment_id, realm)
);
