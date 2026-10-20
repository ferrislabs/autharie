DELETE FROM fleet_audit_log WHERE target_feature IS NOT NULL;

ALTER TABLE fleet_audit_log
    DROP CONSTRAINT fleet_audit_log_names_one_target;

ALTER TABLE fleet_audit_log
    ADD CONSTRAINT fleet_audit_log_names_one_target
        CHECK ((target_dataplane_id IS NULL) != (target_subject IS NULL));

ALTER TABLE fleet_audit_log
    DROP COLUMN target_feature;

DROP TABLE organisation_feature_flags;
DROP TABLE feature_flags;
