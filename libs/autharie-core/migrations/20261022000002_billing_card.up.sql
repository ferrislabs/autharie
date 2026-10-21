ALTER TABLE billing_accounts
    ADD COLUMN card_brand TEXT
        CONSTRAINT billing_accounts_card_brand_shape
        CHECK (card_brand ~ '^[a-z0-9_]{1,32}$'),
    ADD COLUMN card_last4 TEXT
        CONSTRAINT billing_accounts_card_last4_shape
        CHECK (card_last4 ~ '^[0-9]{4}$'),
    ADD COLUMN card_exp_month SMALLINT
        CONSTRAINT billing_accounts_card_exp_month_range
        CHECK (card_exp_month BETWEEN 1 AND 12),
    ADD COLUMN card_exp_year SMALLINT
        CONSTRAINT billing_accounts_card_exp_year_range
        CHECK (card_exp_year BETWEEN 2000 AND 9999),
    ADD CONSTRAINT billing_accounts_card_all_or_nothing
        CHECK (
            (card_brand IS NULL) = (card_last4 IS NULL)
            AND (card_brand IS NULL) = (card_exp_month IS NULL)
            AND (card_brand IS NULL) = (card_exp_year IS NULL)
        );
