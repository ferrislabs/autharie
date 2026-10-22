CREATE TABLE deployment_agents (
    deployment_id UUID PRIMARY KEY REFERENCES deployments(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT false,
    realm TEXT NOT NULL DEFAULT 'master' CHECK (char_length(realm) BETWEEN 1 AND 100),
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    applied_revision BIGINT NOT NULL DEFAULT 0 CHECK (applied_revision >= 0),
    phase TEXT NULL CHECK (phase IS NULL OR phase IN ('Applying', 'Ready', 'Failed')),
    gateway_client_ready BOOLEAN NOT NULL DEFAULT false,
    mcp_url TEXT NULL,
    message TEXT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE agent_policies (
    id UUID PRIMARY KEY,
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    description TEXT NULL CHECK (description IS NULL OR char_length(description) <= 500),
    enabled BOOLEAN NOT NULL,
    rule JSONB NULL,
    cedar TEXT NOT NULL CHECK (char_length(cedar) BETWEEN 1 AND 65536),
    phase TEXT NULL CHECK (phase IS NULL OR phase IN ('Pending', 'Applied', 'Invalid', 'Failed')),
    message TEXT NULL,
    desired_revision BIGINT NOT NULL DEFAULT 0 CHECK (desired_revision >= 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX agent_policies_by_deployment ON agent_policies (deployment_id, created_at, id);
