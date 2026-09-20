import type { FilterStatusValue } from '../types'

export function isUserLocalOnlyRecordStatus(status: FilterStatusValue): boolean {
  return status === 'has_retry' || status === 'has_fallback'
}

export function shouldUseServerUserRecordPagination(input: {
  status: FilterStatusValue
  model: string
  provider: string
  clientFamily: string
  hideUnknownRecords: boolean
}): boolean {
  const hasLocalOnlyFilter = isUserLocalOnlyRecordStatus(input.status)
    || input.model !== '__all__'
    || input.provider !== '__all__'
    || input.clientFamily !== '__all__'
    || input.hideUnknownRecords

  return !hasLocalOnlyFilter
}

// The user API can evaluate `search` itself, so a server-paginated list must not filter the
// returned page again. Local pagination only ever holds the backend's default page, so the
// search box has to be applied on top of it or it would silently do nothing.
export function shouldApplyLocalUserRecordSearch(input: {
  usesServerRecordPagination: boolean
  search: string
}): boolean {
  if (input.usesServerRecordPagination) return false

  return input.search.trim().length > 0
}
