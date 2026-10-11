import { describe, expect, it } from 'vitest'
import { compile, type VNode } from 'vue'
import source from '../MainLayout.vue?raw'
import { userAvatarText, userIdentityLabel } from '@/utils/userIdentity'

// 编译真实模板中的两个头像元素，隔离导航加载、版本检查等无关副作用。
const avatarTemplates = source.match(/<div\b[^>]*>\s*\{\{\s*userAvatarText\([^}]+\)\s*\}\}\s*<\/div>/g) ?? []

describe('MainLayout identity avatars', () => {
  it.each([
    { username: 'zhangsan', display_name: '张三', expected: '张' },
    { username: 'zhangsan', display_name: '𠮷野', expected: '𠮷' },
    { username: 'zhangsan', display_name: null, expected: 'Z' },
  ])('uses one character on desktop and mobile for $display_name', ({ expected, ...user }) => {
    expect(avatarTemplates).toHaveLength(2)
    for (const template of avatarTemplates) {
      const render = compile(template) as (context: Record<string, unknown>, cache: unknown[]) => VNode
      const avatar = render({ authStore: { user }, sidebarCollapsed: false, userAvatarText, userIdentityLabel }, [])
      expect(String(avatar.children).trim()).toBe(expected)
    }
  })
})
