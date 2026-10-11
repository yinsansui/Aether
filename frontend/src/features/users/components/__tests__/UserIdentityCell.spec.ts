import { afterEach, describe, expect, it } from 'vitest'
import { createApp, h, type App } from 'vue'
import UserIdentityCell from '../UserIdentityCell.vue'
import type { UserManagementRow } from '../user-management-types'

const mounts: Array<{ app: App; root: HTMLElement }> = []
afterEach(() => mounts.splice(0).forEach(({ app, root }) => { app.unmount(); root.remove() }))

function mountIdentity(display_name?: string | null) {
  const row = {
    user: { id: 'u1', username: 'zhangsan', display_name, email: 'zhangsan@example.com' },
    roleBadgeVariant: 'secondary', roleLabel: '普通用户',
  } as UserManagementRow
  const root = document.createElement('div')
  const app = createApp({ render: () => h(UserIdentityCell, { row }) })
  document.body.append(root)
  app.mount(root)
  mounts.push({ app, root })
  return root
}

describe('UserIdentityCell', () => {
  it('shows a Chinese name and its login account together', () => {
    const root = mountIdentity('张三')
    expect(root.textContent).toContain('张三')
    expect(root.textContent).toContain('张三（zhangsan）')
    expect(root.textContent).not.toContain('登录账号：')
    expect(root.querySelector('[title="张三（zhangsan）"]')).not.toBeNull()
    expect(root.textContent).toContain('邮箱：zhangsan@example.com')
  })

  it('preserves the existing account display when no name is set', () => {
    const root = mountIdentity(null)
    expect(root.querySelector('[title="zhangsan"]')?.textContent?.trim()).toBe('zhangsan')
    expect(root.textContent).not.toContain('登录账号：')
  })
})
