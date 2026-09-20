import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const source = readFileSync(
  resolve(process.cwd(), 'src/views/shared/Usage.vue'),
  'utf8',
)

function functionBlock(name: string, nextName: string): string {
  return source.split(`async function ${name}`)[1]?.split(`async function ${nextName}`)[0] ?? ''
}

describe('usage record server filters', () => {
  it('prefers server pagination for normal-user records', () => {
    expect(source).toContain('shouldUseServerUserRecordPagination({')
    expect(source).toContain('const usesServerRecordPagination = computed(() => isAdminPage.value || userUsesServerPagination.value)')

    const apiFormatHandler = functionBlock('handleFilterApiFormatChange', 'handleFilterStatusChange')
    const statusHandler = functionBlock('handleFilterStatusChange', 'handleFilterClientFamilyChange')
    expect(apiFormatHandler).toContain('usesServerRecordPagination.value')
    expect(apiFormatHandler).toContain('await loadRecords(')
    expect(statusHandler).toContain('usesServerRecordPagination.value')
    expect(statusHandler).toContain('await loadRecords(')
  })

  it('keeps filtered normal-user pagination and refreshes on the server', () => {
    const pageHandler = functionBlock('handlePageChange', 'handlePageSizeChange')
    const pageSizeHandler = functionBlock('handlePageSizeChange', 'handleFilterSearchChange')
    const refreshHandler = functionBlock('refreshData', 'handleManualRefresh')

    expect(pageHandler).toContain('usesServerRecordPagination.value')
    expect(pageSizeHandler).toContain('usesServerRecordPagination.value')
    expect(refreshHandler).toContain('userUsesServerPagination.value')
    expect(refreshHandler).toContain('await loadRecords(')
  })

  it('keeps retry and fallback filters local because the user API does not accept them', () => {
    expect(source).toContain('shouldUseServerUserRecordPagination({')
    expect(source).toContain('shouldApplyLocalUserRecordSearch({')
    expect(source).toContain('matchesUsageRecordSearch(record, filterSearch.value)')

    const statusHandler = functionBlock('handleFilterStatusChange', 'handleFilterClientFamilyChange')
    expect(statusHandler).toContain('await loadStats(timeRange.value)')
  })

  it('keeps the client-family filter free of admin analytics refreshes', () => {
    const clientFamilyHandler = functionBlock('handleFilterClientFamilyChange', 'refreshData')

    expect(clientFamilyHandler).toContain('usesServerRecordPagination.value')
    expect(clientFamilyHandler).not.toContain('refreshAdminAnalyticsForSelectionChange')
  })
})
