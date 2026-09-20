import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'

const tableAttrs: { current: Record<string, unknown> } = { current: {} }

const usageMocks = vi.hoisted(() => ({
  meGetUsage: vi.fn(),
  meGetActiveRequests: vi.fn(),
  meGetActivityHeatmap: vi.fn(),
  meGetIntervalTimeline: vi.fn(),
}))

vi.mock('vue-router', () => ({
  useRoute: () => ({ path: '/usage' }),
}))

vi.mock('@/stores/auth', () => ({
  useAuthStore: () => ({
    isAdmin: false,
    canAccessAdmin: false,
    canOperateAdmin: false,
    isAuditAdmin: false,
  }),
}))

vi.mock('@/api/me', () => ({
  meApi: {
    getUsage: usageMocks.meGetUsage,
    getActiveRequests: usageMocks.meGetActiveRequests,
    getActivityHeatmap: usageMocks.meGetActivityHeatmap,
    getIntervalTimeline: usageMocks.meGetIntervalTimeline,
  },
}))

vi.mock('@/api/usage', () => ({
  usageApi: {
    getAllUsageRecords: vi.fn(),
    getAllUsageRecordTotal: vi.fn(),
    getUsageStats: vi.fn(),
    getUsageByModel: vi.fn(),
    getUsageByProvider: vi.fn(),
    getUsageByApiFormat: vi.fn(),
    getActiveRequests: vi.fn(),
    getActivityHeatmap: vi.fn(),
  },
}))

vi.mock('@/api/users', () => ({ usersApi: { getAllUsers: vi.fn() } }))
vi.mock('@/api/dashboard', () => ({ dashboardApi: { prefetchRequestDetail: vi.fn() } }))

vi.mock('@/utils/logger', () => ({
  log: { debug: vi.fn(), error: vi.fn(), info: vi.fn(), warn: vi.fn(), http: vi.fn() },
}))

vi.mock('@/features/usage/components', async () => {
  const { defineComponent, h } = await import('vue')
  const stub = (name: string) => defineComponent({
    name,
    inheritAttrs: false,
    setup(_, { attrs }) {
      if (name === 'UsageRecordsTable') {
        tableAttrs.current = attrs as Record<string, unknown>
      }
      return () => h('div', { 'data-stub': name })
    },
  })

  return {
    UsageModelTable: stub('UsageModelTable'),
    UsageProviderTable: stub('UsageProviderTable'),
    UsageApiFormatTable: stub('UsageApiFormatTable'),
    UsageRecordsTable: stub('UsageRecordsTable'),
    ActivityHeatmapCard: stub('ActivityHeatmapCard'),
    RequestDetailDrawer: stub('RequestDetailDrawer'),
    IntervalTimelineCard: stub('IntervalTimelineCard'),
  }
})

import Usage from '../Usage.vue'

async function flushPromises(): Promise<void> {
  await nextTick()
  await Promise.resolve()
  await Promise.resolve()
  await nextTick()
}

function buildUserUsageResponse(records: Array<Record<string, unknown>>, total: number) {
  return {
    total_requests: records.length,
    total_tokens: 0,
    total_cost: 0,
    summary_by_model: [],
    summary_by_api_format: [],
    pagination: { total, limit: records.length, offset: 0, has_more: total > records.length },
    records,
  }
}

function usageCallsWithPagination() {
  return usageMocks.meGetUsage.mock.calls
    .map(([params]) => params as Record<string, unknown>)
    .filter(params => params && typeof params.limit === 'number')
}

function lastUsageCallWithPagination(): Record<string, unknown> | undefined {
  const calls = usageCallsWithPagination()
  return calls[calls.length - 1]
}

function tableProp<T>(name: string): T {
  return tableAttrs.current[name] as T
}

describe('normal-user usage pagination', () => {
  let app: App | null = null
  let teleportTarget: HTMLDivElement | null = null

  beforeEach(() => {
    window.localStorage.clear()
    tableAttrs.current = {}
    vi.clearAllMocks()

    teleportTarget = document.createElement('div')
    teleportTarget.id = 'header-actions-right'
    document.body.appendChild(teleportTarget)

    usageMocks.meGetActiveRequests.mockResolvedValue({ requests: [] })
    usageMocks.meGetActivityHeatmap.mockResolvedValue(null)
    usageMocks.meGetIntervalTimeline.mockResolvedValue({ items: [] })
    usageMocks.meGetUsage.mockResolvedValue(buildUserUsageResponse(
      [
        { id: 'record-1', model: 'gpt-5', provider: 'OpenAI', status: 'completed', created_at: '2026-09-01T00:00:00Z' },
        { id: 'record-2', model: 'claude-needle', provider: 'Anthropic', status: 'completed', created_at: '2026-09-01T00:00:00Z' },
      ],
      5000,
    ))
  })

  afterEach(() => {
    app?.unmount()
    app = null
    teleportTarget?.remove()
    teleportTarget = null
  })

  async function mountUsage() {
    app = createApp(Usage)
    app.mount(document.createElement('div'))
    await flushPromises()
    return tableAttrs
  }

  it('loads the first server page for normal users instead of the default 100 records', async () => {
    const attrs = await mountUsage()

    expect(lastUsageCallWithPagination()).toMatchObject({ limit: 20, offset: 0 })
    expect(tableProp<number>('total-records')).toBe(5000)
    expect(attrs.current.records).toHaveLength(2)
  })

  it('requests the matching server page when the page changes', async () => {
    await mountUsage()

    const onPageChange = tableAttrs.current['onUpdate:currentPage'] as (page: number) => void
    onPageChange(3)
    await flushPromises()

    expect(lastUsageCallWithPagination()).toMatchObject({ limit: 20, offset: 40 })
  })

  it('sends transport filters to the server and resets to the first page', async () => {
    await mountUsage()

    const onStatusChange = tableAttrs.current['onUpdate:filterStatus'] as (status: string) => void
    onStatusChange('websocket')
    await flushPromises()

    expect(lastUsageCallWithPagination()).toMatchObject({
      status: 'websocket',
      limit: 20,
      offset: 0,
    })
  })

  it('applies the search box on top of local-only filter combinations', async () => {
    const attrs = await mountUsage()

    const onModelChange = tableAttrs.current['onUpdate:filterModel'] as (model: string) => void
    onModelChange('claude-needle')
    await flushPromises()

    expect(lastUsageCallWithPagination()).toMatchObject({ limit: 20, offset: 0 })

    const onSearchChange = tableAttrs.current['onUpdate:filterSearch'] as (search: string) => void
    onSearchChange('needle')
    await flushPromises()

    // 本地筛选模式只允许在已加载记录上过滤，不能把搜索框变成无效输入。
    expect(attrs.current.records).toHaveLength(1)
    expect(attrs.current.records).toMatchObject([{ id: 'record-2' }])
  })
})
