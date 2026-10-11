import { Blob as NodeBlob } from 'node:buffer'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App } from 'vue'
import AuditLogs from '../AuditLogs.vue'
import { parseCsv } from '@/tests/helpers/parseCsv'

const getAuditLogs = vi.hoisted(() => vi.fn())
vi.mock('@/api/audit', () => ({ auditApi: { getAuditLogs } }))
vi.mock('@/components/ui', async () => {
  const { defineComponent, h } = await import('vue')
  const container = defineComponent({ setup: (_, { slots }) => () => h('div', slots.default?.()) })
  const button = defineComponent({ setup: (_, { slots }) => () => h('button', slots.default?.()) })
  return {
    Card: container, Button: button, Badge: container, Separator: container, Label: container,
    Select: container, SelectTrigger: container, SelectValue: container, SelectContent: container,
    SelectItem: container, Table: container, TableHeader: container, TableBody: container,
    TableRow: container, TableHead: container, SortableTableHead: container, TableFilterMenu: container,
    TableCell: container, Input: container, Pagination: container, RefreshButton: button,
  }
})

let app: App | undefined
let root: HTMLElement | undefined
afterEach(() => {
  app?.unmount()
  root?.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function flushPromises() {
  await new Promise(resolve => setTimeout(resolve, 0))
  await nextTick()
}

describe('AuditLogs CSV export', () => {
  it('round-trips quoted names and every text column without shifting audit fields', async () => {
    const record = {
      id: 'audit-1', created_at: '2026-10-09T12:00:00Z', user_email: 'name"tag,mail@example.com',
      user_display_name: '张"三,测试', user_username: 'zhangsan', user_id: 'user-1',
      event_type: 'login_success', description: '描述含"引号",逗号\n第二行',
      ip_address: '127.0.0.1', status_code: 200, error_message: '错误含"引号"\r\n详细原因',
    }
    getAuditLogs.mockResolvedValue({ items: [record], meta: { total: 1 } })
    let exported: NodeBlob | undefined
    vi.stubGlobal('Blob', NodeBlob)
    vi.stubGlobal('URL', Object.assign(class extends URL {}, {
      createObjectURL(blob: NodeBlob) { exported = blob; return 'blob:audit-test' },
    }))
    const download = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {})
    root = document.createElement('div')
    document.body.append(root)
    app = createApp(defineComponent({ setup: () => () => h(AuditLogs) }))
    app.mount(root)
    await flushPromises()
    root.querySelector<HTMLButtonElement>('button[title="导出"]')!.click()
    await flushPromises()

    expect(download).toHaveBeenCalledOnce()
    expect(exported).toBeDefined()
    const rows = parseCsv(await exported!.text())
    expect(rows[0]).toEqual(['时间', '用户邮箱', '用户姓名', '登录账号', '用户ID', '事件类型', '描述', 'IP地址', '状态码', '错误消息'])
    expect(rows[1]).toEqual([
      record.created_at, record.user_email, record.user_display_name, record.user_username,
      record.user_id, record.event_type, record.description, record.ip_address,
      String(record.status_code), record.error_message,
    ])
    expect(rows).toHaveLength(2)
    expect(rows[1]).toHaveLength(10)
  })
})
