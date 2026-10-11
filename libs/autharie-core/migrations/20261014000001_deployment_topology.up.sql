ALTER TABLE deployments
    ADD COLUMN replicas SMALLINT NOT NULL DEFAULT 1
        CONSTRAINT deployments_replicas_in_range CHECK (replicas BETWEEN 1 AND 5),
    ADD COLUMN database_instances SMALLINT NOT NULL DEFAULT 1
        CONSTRAINT deployments_database_instances_in_range CHECK (database_instances BETWEEN 1 AND 5);
