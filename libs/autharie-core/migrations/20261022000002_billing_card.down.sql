ALTER TABLE billing_accounts
    DROP CONSTRAINT billing_accounts_card_all_or_nothing,
    DROP COLUMN card_exp_year,
    DROP COLUMN card_exp_month,
    DROP COLUMN card_last4,
    DROP COLUMN card_brand;
