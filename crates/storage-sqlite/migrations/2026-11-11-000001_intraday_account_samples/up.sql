-- Scheduled account valuation observations used only to shape short-range
-- intraday charts. Daily valuation history remains the source of truth.
CREATE TABLE intraday_account_samples (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    timestamp INTEGER NOT NULL,
    base_currency TEXT NOT NULL,
    total_value_base TEXT NOT NULL,
    net_contribution_base TEXT NOT NULL,
    PRIMARY KEY (account_id, timestamp, base_currency)
);

CREATE INDEX idx_intraday_account_samples_window
    ON intraday_account_samples (base_currency, timestamp, account_id);
