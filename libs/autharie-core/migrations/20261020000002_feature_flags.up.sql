CREATE TABLE feature_flags (
    key                TEXT        PRIMARY KEY,
    enabled_by_default BOOLEAN     NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL,
    updated_by         TEXT        NOT NULL
);

CREATE TABLE organisation_feature_flags (
    organisation_id UUID        NOT NULL REFERENCES organisations(id) ON DELETE CASCADE,
    key             TEXT        NOT NULL,
    enabled         BOOLEAN     NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    updated_by      TEXT        NOT NULL,

    PRIMARY KEY (organisation_id, key)
);

ALTER TABLE fleet_audit_log
    ADD COLUMN target_feature TEXT;

ALTER TABLE fleet_audit_log
    DROP CONSTRAINT fleet_audit_log_names_one_target;

ALTER TABLE fleet_audit_log
    ADD CONSTRAINT fleet_audit_log_names_one_target
        CHECK (num_nonnulls(target_dataplane_id, target_subject, target_feature) = 1);
