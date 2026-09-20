import { describe, expect, it } from 'vitest'

import {
  isUserLocalOnlyRecordStatus,
  shouldApplyLocalUserRecordSearch,
  shouldUseServerUserRecordPagination,
} from '../recordFilterPolicy'

describe('normal-user usage record filter policy', () => {
  const baseFilters = {
    status: '__all__' as const,
    model: '__all__',
    provider: '__all__',
    clientFamily: '__all__',
    hideUnknownRecords: false,
  }

  it('uses server pagination for the unfiltered normal-user list', () => {
    expect(shouldUseServerUserRecordPagination(baseFilters)).toBe(true)
  })

  it('uses server pagination for filters supported by the user API', () => {
    expect(shouldUseServerUserRecordPagination({
      ...baseFilters,
      status: 'websocket',
    })).toBe(true)
  })

  it.each([
    { status: 'has_retry' as const },
    { model: 'gpt-5' },
    { provider: 'OpenAI' },
    { clientFamily: 'codex' },
    { hideUnknownRecords: true },
  ])('keeps local-only filter combination %# on local pagination', (override) => {
    expect(shouldUseServerUserRecordPagination({
      ...baseFilters,
      ...override,
    })).toBe(false)
  })

  it.each(['has_retry', 'has_fallback'] as const)('identifies %s as a local-only status', (status) => {
    expect(isUserLocalOnlyRecordStatus(status)).toBe(true)
  })
})

describe('normal-user usage record search policy', () => {
  it('leaves search to the server when the list is server paginated', () => {
    expect(shouldApplyLocalUserRecordSearch({
      usesServerRecordPagination: true,
      search: 'production',
    })).toBe(false)
  })

  it('filters locally for local-pagination filter combinations', () => {
    expect(shouldApplyLocalUserRecordSearch({
      usesServerRecordPagination: false,
      search: 'production',
    })).toBe(true)
  })

  it('does not filter locally when the search box is empty', () => {
    expect(shouldApplyLocalUserRecordSearch({
      usesServerRecordPagination: false,
      search: '   ',
    })).toBe(false)
  })
})
