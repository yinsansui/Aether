-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_usage_settlement_wallet_daily_finalized
ON public.usage_settlement_snapshots (finalized_at)
WHERE billing_status = 'settled' AND wallet_id IS NOT NULL;
