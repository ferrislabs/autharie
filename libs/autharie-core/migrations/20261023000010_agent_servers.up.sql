CREATE TABLE agent_servers (
    id UUID PRIMARY KEY,
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    slug TEXT NOT NULL CHECK (slug ~ '^[a-z][a-z0-9-]{1,30}$' AND slug NOT IN ('ferriskey', 'autharie')),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    url TEXT NOT NULL CHECK (char_length(url) BETWEEN 1 AND 2048),
    enabled BOOLEAN NOT NULL,
    discovery JSONB NULL,
    discovery_requested_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX agent_servers_slug_per_deployment ON agent_servers (deployment_id, slug);
CREATE INDEX agent_servers_by_deployment ON agent_servers (deployment_id, created_at, id);
