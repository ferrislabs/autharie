DROP TABLE cluster_inventory;

DELETE FROM data_planes WHERE mode = 'customer';

DROP INDEX idx_data_planes_customer_credential;

ALTER TABLE data_planes
    DROP CONSTRAINT data_planes_failure_reason_only_when_failed,
    DROP CONSTRAINT data_planes_owner_by_mode,
    DROP COLUMN failure_reason,
    DROP COLUMN credential_id,
    DROP COLUMN deployment_id,
    ADD CONSTRAINT data_planes_dedicated_has_owner
        CHECK (
            (mode = 'dedicated' AND organisation_id IS NOT NULL)
            OR (mode <> 'dedicated' AND organisation_id IS NULL)
        );

DROP INDEX idx_deployments_credential;

ALTER TABLE deployments
    DROP CONSTRAINT deployments_customer_cloud_is_whole,
    DROP CONSTRAINT deployments_distribution_known,
    DROP COLUMN cluster_profile,
    DROP COLUMN credential_id,
    DROP COLUMN distribution;

DROP TABLE cloud_credentials;
DROP TABLE cloud_credential_secrets;
