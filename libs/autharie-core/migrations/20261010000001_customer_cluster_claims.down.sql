DROP INDEX idx_data_planes_customer_to_provision;

ALTER TABLE data_planes
    DROP COLUMN provisioning_claimed_until;
