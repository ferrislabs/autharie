CREATE TABLE platform_cells_settings (
    id              BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (id),
    organisation_id UUID        NULL REFERENCES organisations (id) ON DELETE SET NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by      TEXT
);

INSERT INTO platform_cells_settings (id) VALUES (TRUE);
