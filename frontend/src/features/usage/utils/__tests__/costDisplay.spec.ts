import { describe, expect, it } from 'vitest'

import { resolveCostDisplay } from '../costDisplay'

describe('usage cost display', () => {
  it('prefers the charged amount over the standard price', () => {
    expect(resolveCostDisplay(0.1, 0.02)).toEqual({ primary: 0.02, standard: 0.1 })
  })

  it('keeps a zero charged amount instead of falling back to the standard price', () => {
    expect(resolveCostDisplay(0.1, 0)).toEqual({ primary: 0, standard: 0.1 })
  })

  it('hides the standard price when both amounts are equal', () => {
    expect(resolveCostDisplay(0.1, 0.1)).toEqual({ primary: 0.1 })
  })

  it('falls back to the standard price for rows without a charged amount', () => {
    expect(resolveCostDisplay(0.1, undefined)).toEqual({ primary: 0.1 })
    expect(resolveCostDisplay(0.1, null)).toEqual({ primary: 0.1 })
    expect(resolveCostDisplay(undefined, undefined)).toEqual({ primary: 0 })
  })
})
