ALTER TABLE deployments
    DROP CONSTRAINT deployments_database_instances_in_range,
    DROP CONSTRAINT deployments_replicas_in_range,
    DROP COLUMN database_instances,
    DROP COLUMN replicas;
