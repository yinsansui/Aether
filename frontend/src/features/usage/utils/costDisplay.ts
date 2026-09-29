/**
 * Cost display helpers.
 *
 * `standard` is the list price (catalog price after channel override and time
 * pricing multiplier), `actual` is what the wallet actually debits after the
 * API key rate multiplier is applied.
 *
 * The charged amount is the primary value users see; the standard price is kept
 * as a secondary reference when the two differ.
 */
export interface CostDisplay {
  /** Primary value rendered as the request/aggregate cost. */
  primary: number
  /** Standard price, only present when it differs from the charged amount. */
  standard?: number
}

function toFiniteNumber(value: number | null | undefined): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

export function resolveCostDisplay(
  standard: number | null | undefined,
  actual: number | null | undefined,
): CostDisplay {
  const standardValue = toFiniteNumber(standard) ?? 0
  const actualValue = toFiniteNumber(actual)

  // A charged amount of 0 is legitimate (free tier), so this must be an explicit
  // null/undefined check rather than a falsy fallback.
  if (actualValue === undefined) {
    return { primary: standardValue }
  }

  return {
    primary: actualValue,
    standard: actualValue === standardValue ? undefined : standardValue,
  }
}
