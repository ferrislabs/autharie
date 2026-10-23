CREATE TABLE deployment_portal_theme_sync (
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    realm TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    phase TEXT NOT NULL DEFAULT 'pending' CHECK (phase IN ('pending', 'applied', 'failed')),
    message TEXT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (deployment_id, realm)
);

CREATE TABLE deployment_portal_themes (
    id UUID PRIMARY KEY,
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    realm TEXT NOT NULL,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    name_key TEXT GENERATED ALWAYS AS (lower(name)) STORED,
    tokens JSONB NOT NULL
        CHECK (jsonb_typeof(tokens) = 'object' AND octet_length(tokens::text) <= 32768),
    active BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT deployment_portal_themes_name_unique UNIQUE (deployment_id, realm, name_key)
);

CREATE UNIQUE INDEX deployment_portal_themes_one_active
    ON deployment_portal_themes (deployment_id, realm)
    WHERE active;
