CREATE TABLE agent_access (
    deployment_id UUID PRIMARY KEY REFERENCES deployments(id) ON DELETE CASCADE,
    approval_required BOOLEAN NOT NULL DEFAULT false,
    approved TEXT[] NOT NULL DEFAULT '{}',
    blocked TEXT[] NOT NULL DEFAULT '{}',
    deleted TEXT[] NOT NULL DEFAULT '{}',
    clients_truncated BOOLEAN NOT NULL DEFAULT false,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE agent_clients (
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL CHECK (char_length(client_id) BETWEEN 1 AND 512),
    name TEXT NULL CHECK (name IS NULL OR char_length(name) <= 100),
    source TEXT NOT NULL CHECK (source IN ('dynamic', 'metadata_document', 'autharie')),
    kind TEXT NOT NULL CHECK (kind IN ('delegated', 'autonomous')),
    enabled BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NULL,
    reported_at TIMESTAMPTZ NOT NULL,
    position INT NOT NULL DEFAULT 0,
    PRIMARY KEY (deployment_id, client_id)
);

CREATE TABLE agent_autonomous (
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL CHECK (char_length(client_id) BETWEEN 1 AND 512),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    description TEXT NULL CHECK (description IS NULL OR char_length(description) <= 500),
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (deployment_id, client_id)
);

CREATE TABLE agent_credentials (
    request_id UUID PRIMARY KEY,
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL CHECK (char_length(client_id) BETWEEN 1 AND 512),
    kind TEXT NOT NULL CHECK (kind IN ('create', 'rotate')),
    sealing_key JSONB NOT NULL,
    sealed TEXT NULL CHECK (sealed IS NULL OR char_length(sealed) <= 4096),
    delivered_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    CHECK (delivered_at IS NULL OR sealed IS NULL)
);

CREATE INDEX agent_credentials_by_deployment ON agent_credentials (deployment_id, expires_at);
CREATE INDEX agent_credentials_by_expiry ON agent_credentials (expires_at);
