CREATE TABLE iam_requests (
    request_id UUID PRIMARY KEY,
    deployment_id UUID NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
    realm TEXT NOT NULL CHECK (char_length(realm) BETWEEN 1 AND 255),
    operation TEXT NOT NULL CHECK (operation IN ('applications.list', 'applications.get')),
    actor TEXT NOT NULL CHECK (char_length(actor) BETWEEN 1 AND 512),
    phase TEXT NOT NULL DEFAULT 'pending' CHECK (phase IN ('pending', 'done', 'failed', 'delivered')),
    envelope JSONB NULL,
    message TEXT NULL CHECK (message IS NULL OR char_length(message) <= 500),
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    CHECK ((phase = 'done') = (envelope IS NOT NULL)),
    CHECK (phase = 'failed' OR message IS NULL)
);

CREATE INDEX iam_requests_by_deployment ON iam_requests (deployment_id, expires_at);
CREATE INDEX iam_requests_by_expiry ON iam_requests (expires_at);
