-- Fail before queuing behind active traffic or holding a lock while waiting for another.
LOCK TABLE public.usage, public.usage_settlement_snapshots IN SHARE ROW EXCLUSIVE MODE NOWAIT;
LOCK TABLE public.stats_hourly_user, public.stats_hourly IN ACCESS EXCLUSIVE MODE NOWAIT;

ALTER TABLE public.stats_hourly_user
    ADD COLUMN IF NOT EXISTS total_tokens bigint;

ALTER TABLE public.stats_hourly
    ADD COLUMN IF NOT EXISTS user_totals_row_count bigint;

CREATE FUNCTION public.stats_hourly_user_totals_lock_key(target_hour timestamptz)
RETURNS bigint LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT hashtextextended('aether:stats_hourly_user_totals', 0)
        # (EXTRACT(EPOCH FROM target_hour)::bigint / 3600)
$$;

CREATE FUNCTION public.stats_hourly_user_totals_delete_lock_key()
RETURNS bigint LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT hashtextextended('aether:stats_hourly_user_totals_delete', 0)
$$;

-- A change can commit after the hourly task's read, even when the bucket did not
-- exist at the start of the change. Coordinate publication and invalidation by hour.
CREATE FUNCTION public.invalidate_stats_hourly_user_totals(target_hour timestamptz, deleting boolean)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    IF deleting THEN
        -- Retention and full purges can delete facts from many different hours.
        PERFORM pg_advisory_xact_lock_shared(public.stats_hourly_user_totals_delete_lock_key());
    ELSE
        PERFORM pg_advisory_xact_lock_shared(public.stats_hourly_user_totals_lock_key(target_hour));
    END IF;

    IF EXISTS (SELECT 1 FROM public.stats_hourly WHERE hour_utc = target_hour AND user_totals_row_count IS NOT NULL) THEN
        -- Late updates can affect multiple published hours in different orders.
        PERFORM pg_advisory_xact_lock(hashtextextended('aether:stats_hourly_user_totals_invalidate', 0));
        UPDATE public.stats_hourly
        SET user_totals_row_count = NULL
        WHERE hour_utc = target_hour AND user_totals_row_count IS NOT NULL;
    END IF;
END;
$$;

CREATE FUNCTION public.invalidate_stats_hourly_user_totals_from_usage()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    old_hour timestamptz;
    new_hour timestamptz;
BEGIN
    IF TG_OP = 'UPDATE' AND ROW(
        OLD.created_at, OLD.user_id, OLD.status, OLD.provider_name,
        OLD.input_tokens, OLD.output_tokens, OLD.total_tokens, OLD.api_format, OLD.endpoint_api_format,
        OLD.cache_creation_input_tokens, OLD.cache_creation_input_tokens_5m,
        OLD.cache_creation_input_tokens_1h, OLD.cache_read_input_tokens
    ) IS NOT DISTINCT FROM ROW(
        NEW.created_at, NEW.user_id, NEW.status, NEW.provider_name,
        NEW.input_tokens, NEW.output_tokens, NEW.total_tokens, NEW.api_format, NEW.endpoint_api_format,
        NEW.cache_creation_input_tokens, NEW.cache_creation_input_tokens_5m,
        NEW.cache_creation_input_tokens_1h, NEW.cache_read_input_tokens
    ) THEN
        RETURN NULL;
    END IF;

    IF TG_OP <> 'INSERT' AND OLD.user_id IS NOT NULL
        AND OLD.status NOT IN ('pending', 'streaming') AND OLD.provider_name NOT IN ('unknown', 'pending') THEN
        old_hour := date_trunc('hour', OLD.created_at, 'UTC');
    END IF;
    IF TG_OP <> 'DELETE' AND NEW.user_id IS NOT NULL
        AND NEW.status NOT IN ('pending', 'streaming') AND NEW.provider_name NOT IN ('unknown', 'pending') THEN
        new_hour := date_trunc('hour', NEW.created_at, 'UTC');
    END IF;

    PERFORM public.invalidate_stats_hourly_user_totals(affected.hour_utc, TG_OP = 'DELETE')
    FROM (SELECT old_hour AS hour_utc UNION SELECT new_hour) AS affected
    WHERE affected.hour_utc IS NOT NULL
    ORDER BY affected.hour_utc;
    RETURN NULL;
END;
$$;

-- Lock statistics rows at commit, after the transaction has finished changing facts.
CREATE CONSTRAINT TRIGGER invalidate_stats_hourly_user_totals_from_usage
AFTER INSERT OR DELETE OR UPDATE OF created_at, user_id, status, provider_name, input_tokens,
    output_tokens, total_tokens, api_format, endpoint_api_format, cache_creation_input_tokens,
    cache_creation_input_tokens_5m, cache_creation_input_tokens_1h, cache_read_input_tokens
ON public.usage DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION public.invalidate_stats_hourly_user_totals_from_usage();

CREATE FUNCTION public.invalidate_stats_hourly_user_totals_from_settlement()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF ROW(
        OLD.billing_effective_input_tokens, OLD.billing_output_tokens,
        OLD.billing_cache_creation_tokens, OLD.billing_cache_creation_5m_tokens,
        OLD.billing_cache_creation_1h_tokens, OLD.billing_cache_read_tokens, OLD.billing_total_input_context
    ) IS NOT DISTINCT FROM ROW(
        NEW.billing_effective_input_tokens, NEW.billing_output_tokens,
        NEW.billing_cache_creation_tokens, NEW.billing_cache_creation_5m_tokens,
        NEW.billing_cache_creation_1h_tokens, NEW.billing_cache_read_tokens, NEW.billing_total_input_context
    ) AND (TG_OP <> 'UPDATE' OR OLD.request_id = NEW.request_id) THEN
        RETURN NULL;
    END IF;

    PERFORM public.invalidate_stats_hourly_user_totals(affected.hour_utc, TG_OP = 'DELETE')
    FROM (
        SELECT DISTINCT date_trunc('hour', usage.created_at, 'UTC') AS hour_utc
        FROM public.usage
        WHERE usage.request_id IN (OLD.request_id, NEW.request_id)
    ) AS affected
    ORDER BY affected.hour_utc;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER invalidate_stats_hourly_user_totals_from_settlement
AFTER INSERT OR DELETE OR UPDATE OF request_id, billing_effective_input_tokens, billing_output_tokens,
    billing_cache_creation_tokens, billing_cache_creation_5m_tokens, billing_cache_creation_1h_tokens,
    billing_cache_read_tokens, billing_total_input_context
ON public.usage_settlement_snapshots DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION public.invalidate_stats_hourly_user_totals_from_settlement();
