-- The stats rollups that break usage down by model/provider/api-key only ever
-- carried the standard catalog price (`total_cost`). They now also mirror the
-- billed amount (`actual_total_cost`) so read paths can report what users were
-- really charged without falling back to a full scan of `usage_billing_facts`.
--
-- `total_cost` semantics are unchanged: it is still the catalog price with the
-- time-window multiplier applied, before the API key rate multiplier.

ALTER TABLE public.stats_daily_model
    ADD COLUMN IF NOT EXISTS actual_total_cost numeric(20,8) DEFAULT '0'::double precision NOT NULL;

ALTER TABLE public.stats_daily_provider
    ADD COLUMN IF NOT EXISTS actual_total_cost numeric(20,8) DEFAULT '0'::double precision NOT NULL;

ALTER TABLE public.stats_daily_api_key
    ADD COLUMN IF NOT EXISTS actual_total_cost numeric(20,8) DEFAULT '0'::double precision NOT NULL;

ALTER TABLE public.stats_daily_model_provider
    ADD COLUMN IF NOT EXISTS actual_total_cost numeric(20,8) DEFAULT '0'::double precision NOT NULL;

ALTER TABLE public.stats_user_daily_model_provider
    ADD COLUMN IF NOT EXISTS actual_total_cost numeric(20,8) DEFAULT '0'::double precision NOT NULL;
