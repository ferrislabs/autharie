CREATE TABLE organisation_offer_overrides (
    organisation_id UUID        NOT NULL REFERENCES organisations(id) ON DELETE CASCADE,
    offer           TEXT        NOT NULL,
    open            BOOLEAN     NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    updated_by      TEXT        NOT NULL,

    PRIMARY KEY (organisation_id, offer)
);
