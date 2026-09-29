-- Populate the billed-cost mirror added to the model/provider/api-key rollups.
-- Historical rows were written before the column existed, so without this pass
-- every day already rolled up would report a zero billed amount.
--
-- Only `actual_total_cost` is refreshed; request/token/standard-price columns are
-- left untouched. Source and filters match the daily rollup writer.

WITH aggregated AS (
    SELECT
        (date_trunc('day', usage.created_at AT TIME ZONE 'UTC') AT TIME ZONE 'UTC') AS day_utc,
        usage.model,
        COALESCE(SUM(COALESCE(usage.actual_total_cost_usd, 0)), 0)::DOUBLE PRECISION
            AS actual_total_cost
    FROM usage_billing_facts AS usage
    WHERE usage.model IS NOT NULL
      AND usage.model <> ''
      AND usage.status NOT IN ('pending', 'streaming')
      AND usage.provider_name NOT IN ('unknown', 'pending')
    GROUP BY 1, 2
)
UPDATE public.stats_daily_model AS target
SET actual_total_cost = aggregated.actual_total_cost,
    updated_at = NOW()
FROM aggregated
WHERE target.date = aggregated.day_utc
  AND target.model = aggregated.model;

WITH aggregated AS (
    SELECT
        (date_trunc('day', usage.created_at AT TIME ZONE 'UTC') AT TIME ZONE 'UTC') AS day_utc,
        COALESCE(usage.provider_name, 'Unknown') AS provider_name,
        COALESCE(SUM(COALESCE(usage.actual_total_cost_usd, 0)), 0)::DOUBLE PRECISION
            AS actual_total_cost
    FROM usage_billing_facts AS usage
    WHERE usage.status NOT IN ('pending', 'streaming')
      AND usage.provider_name NOT IN ('unknown', 'pending')
    GROUP BY 1, 2
)
UPDATE public.stats_daily_provider AS target
SET actual_total_cost = aggregated.actual_total_cost,
    updated_at = NOW()
FROM aggregated
WHERE target.date = aggregated.day_utc
  AND target.provider_name = aggregated.provider_name;

WITH aggregated AS (
    SELECT
        (date_trunc('day', usage.created_at AT TIME ZONE 'UTC') AT TIME ZONE 'UTC') AS day_utc,
        usage.api_key_id,
        COALESCE(SUM(COALESCE(usage.actual_total_cost_usd, 0)), 0)::DOUBLE PRECISION
            AS actual_total_cost
    FROM usage_billing_facts AS usage
    WHERE usage.api_key_id IS NOT NULL
    GROUP BY 1, 2
)
UPDATE public.stats_daily_api_key AS target
SET actual_total_cost = aggregated.actual_total_cost,
    updated_at = NOW()
FROM aggregated
WHERE target.date = aggregated.day_utc
  AND target.api_key_id = aggregated.api_key_id;

WITH aggregated AS (
    SELECT
        (date_trunc('day', usage.created_at AT TIME ZONE 'UTC') AT TIME ZONE 'UTC') AS day_utc,
        usage.model,
        usage.provider_name,
        COALESCE(SUM(COALESCE(usage.actual_total_cost_usd, 0)), 0)::DOUBLE PRECISION
            AS actual_total_cost
    FROM usage_billing_facts AS usage
    WHERE usage.model IS NOT NULL
      AND usage.model <> ''
      AND usage.provider_name IS NOT NULL
      AND usage.provider_name <> ''
      AND usage.status NOT IN ('pending', 'streaming')
      AND usage.provider_name NOT IN ('unknown', 'pending')
    GROUP BY 1, 2, 3
)
UPDATE public.stats_daily_model_provider AS target
SET actual_total_cost = aggregated.actual_total_cost,
    updated_at = NOW()
FROM aggregated
WHERE target.date = aggregated.day_utc
  AND target.model = aggregated.model
  AND target.provider_name = aggregated.provider_name;

WITH aggregated AS (
    SELECT
        (date_trunc('day', usage.created_at AT TIME ZONE 'UTC') AT TIME ZONE 'UTC') AS day_utc,
        usage.user_id,
        usage.model,
        usage.provider_name,
        COALESCE(SUM(COALESCE(usage.actual_total_cost_usd, 0)), 0)::DOUBLE PRECISION
            AS actual_total_cost
    FROM usage_billing_facts AS usage
    WHERE usage.user_id IS NOT NULL
      AND usage.model IS NOT NULL
      AND usage.model <> ''
      AND usage.provider_name IS NOT NULL
      AND usage.provider_name <> ''
      AND usage.status NOT IN ('pending', 'streaming')
      AND usage.provider_name NOT IN ('unknown', 'pending')
    GROUP BY 1, 2, 3, 4
)
UPDATE public.stats_user_daily_model_provider AS target
SET actual_total_cost = aggregated.actual_total_cost,
    updated_at = NOW()
FROM aggregated
WHERE target.date = aggregated.day_utc
  AND target.user_id = aggregated.user_id
  AND target.model = aggregated.model
  AND target.provider_name = aggregated.provider_name;
