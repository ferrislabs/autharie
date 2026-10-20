CREATE EXTENSION IF NOT EXISTS btree_gist;

CREATE TABLE billing_tiers (
    code TEXT PRIMARY KEY
        CONSTRAINT billing_tiers_code_shape
        CHECK (code ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND length(code) <= 64),
    name TEXT NOT NULL,
    monthly_price_minor BIGINT NOT NULL
        CONSTRAINT billing_tiers_price_not_negative CHECK (monthly_price_minor >= 0),
    offer TEXT NOT NULL
        CONSTRAINT billing_tiers_offer_known
        CHECK (offer IN ('free', 'sandbox', 'standard', 'scale', 'private')),
    features TEXT[] NOT NULL,
    engines TEXT[] NOT NULL,
    hostings TEXT[] NOT NULL,
    selectable BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE billing_subscriptions (
    id UUID PRIMARY KEY,
    organisation_id UUID NOT NULL,
    deployment_id UUID NOT NULL,
    tier_code TEXT NOT NULL REFERENCES billing_tiers (code),
    monthly_price_minor BIGINT NOT NULL
        CONSTRAINT billing_subscriptions_price_not_negative CHECK (monthly_price_minor >= 0),
    offer TEXT NOT NULL,
    features TEXT[] NOT NULL,
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ
        CONSTRAINT billing_subscriptions_ends_after_start
        CHECK (ends_at IS NULL OR ends_at >= starts_at),
    validity TSTZRANGE GENERATED ALWAYS AS (tstzrange(starts_at, ends_at, '[)')) STORED,
    CONSTRAINT billing_subscriptions_one_at_a_time
        EXCLUDE USING gist (deployment_id WITH =, validity WITH &&)
);

CREATE INDEX idx_billing_subscriptions_organisation
    ON billing_subscriptions (organisation_id);

CREATE INDEX idx_billing_subscriptions_deployment
    ON billing_subscriptions (deployment_id, starts_at DESC);

CREATE TABLE billing_accounts (
    organisation_id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL,
    address_line1 TEXT NOT NULL,
    address_line2 TEXT,
    postal_code TEXT NOT NULL,
    city TEXT NOT NULL,
    country TEXT NOT NULL
        CONSTRAINT billing_accounts_country_shape CHECK (country ~ '^[A-Z]{2}$'),
    vat_number TEXT,
    siren TEXT
        CONSTRAINT billing_accounts_siren_shape CHECK (siren ~ '^[0-9]{9}$'),
    provider_customer_id TEXT UNIQUE,
    payment_method TEXT NOT NULL DEFAULT 'missing'
        CONSTRAINT billing_accounts_payment_method_known
        CHECK (payment_method IN ('missing', 'valid', 'failing')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE billing_statements (
    id UUID PRIMARY KEY,
    organisation_id UUID NOT NULL,
    period TEXT NOT NULL
        CONSTRAINT billing_statements_period_shape
        CHECK (period ~ '^[0-9]{4}-(0[1-9]|1[0-2])$'),
    status TEXT NOT NULL
        CONSTRAINT billing_statements_status_known
        CHECK (status IN (
            'zero', 'held', 'draft', 'sent', 'paid', 'payment_failed', 'uncollectible'
        )),
    total_minor BIGINT NOT NULL,
    provider_invoice_id TEXT UNIQUE,
    hosted_invoice_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT billing_statements_one_per_period UNIQUE (organisation_id, period)
);

CREATE TABLE billing_statement_lines (
    statement_id UUID NOT NULL REFERENCES billing_statements (id) ON DELETE CASCADE,
    position INT NOT NULL,
    deployment_id UUID NOT NULL,
    subscription_id UUID NOT NULL REFERENCES billing_subscriptions (id),
    tier_code TEXT NOT NULL,
    monthly_price_minor BIGINT NOT NULL,
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ NOT NULL,
    covered_seconds BIGINT NOT NULL
        CONSTRAINT billing_statement_lines_covered_not_negative CHECK (covered_seconds >= 0),
    amount_minor BIGINT NOT NULL,
    PRIMARY KEY (statement_id, position)
);

CREATE TABLE billing_provider_events (
    id TEXT PRIMARY KEY,
    received_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE billing_suspensions (
    id UUID PRIMARY KEY,
    organisation_id UUID NOT NULL,
    statement_id UUID NOT NULL REFERENCES billing_statements (id),
    stopped_deployments UUID[] NOT NULL DEFAULT '{}',
    suspended_organisation BOOLEAN NOT NULL,
    suspended_at TIMESTAMPTZ NOT NULL,
    lifted_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX billing_suspensions_one_active
    ON billing_suspensions (organisation_id)
    WHERE lifted_at IS NULL;

INSERT INTO billing_tiers
    (code, name, monthly_price_minor, offer, features, engines, hostings, selectable)
SELECT
    'legacy-' || offer,
    initcap(offer) || ' (legacy)',
    0,
    offer,
    ARRAY[
        'sso_connectors', 'mfa', 'directory_federation', 'custom_domain',
        'branding', 'analytics', 'compliance', 'delegated_admin'
    ],
    ARRAY['ferriskey', 'keycloak'],
    ARRAY['shared', 'self_hosted', 'customer_cloud'],
    false
FROM unnest(ARRAY['free', 'sandbox', 'standard', 'scale', 'private']) AS offer;

INSERT INTO billing_subscriptions
    (id, organisation_id, deployment_id, tier_code, monthly_price_minor, offer, features,
     starts_at, ends_at)
SELECT
    gen_random_uuid(),
    d.organisation_id,
    d.id,
    t.code,
    0,
    t.offer,
    t.features,
    now(),
    NULL
FROM deployments d
JOIN data_planes p ON p.id = d.dataplane_id
CROSS JOIN LATERAL (
    SELECT COALESCE(d.offer, CASE WHEN p.mode = 'shared' THEN 'sandbox' ELSE 'private' END)
        AS offer
) chosen
LEFT JOIN billing_tiers t ON t.code = 'legacy-' || chosen.offer
WHERE d.deleted_at IS NULL;
