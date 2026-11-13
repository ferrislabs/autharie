-- One row per override platform staff set for an organisation. A missing row
-- means the default of each deployment's offer applies; a NULL value is an
-- override to "no limit", which is not the same thing.
CREATE TABLE organisation_quotas (
    organisation_id UUID        NOT NULL REFERENCES organisations(id) ON DELETE CASCADE,
    kind            TEXT        NOT NULL,
    max_count       INTEGER     CHECK (max_count >= 0),
    updated_at      TIMESTAMPTZ NOT NULL,
    updated_by      TEXT        NOT NULL,

    PRIMARY KEY (organisation_id, kind)
);
