-- no-transaction
-- A legacy usage row's billing status can differ from its authoritative snapshot.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_usage_finalized_at
ON public.usage (finalized_at);
