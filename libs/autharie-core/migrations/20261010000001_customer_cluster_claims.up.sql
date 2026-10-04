ALTER TABLE data_planes
    ADD COLUMN provisioning_claimed_until TIMESTAMPTZ NULL;

CREATE INDEX idx_data_planes_customer_to_provision
    ON data_planes (created_at)
    WHERE mode = 'customer' AND status = 'provisioning' AND herald_client_id IS NULL;
