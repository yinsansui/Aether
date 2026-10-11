import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/config/demo', () => ({
  isDemoMode: () => true,
  DEMO_ACCOUNTS: {
    admin: { email: 'admin@demo.aether.io', password: 'demo123' },
    user: { email: 'user@demo.aether.io', password: 'demo123' },
  },
}))

import { handleMockRequest, setMockUserToken } from '../handler'
import { MOCK_ADMIN_PROFILE, MOCK_ADMIN_USER, MOCK_ALL_USERS, MOCK_NORMAL_USER, MOCK_USER_PROFILE } from '../data'

afterEach(() => {
  MOCK_ADMIN_USER.display_name = '演示管理员'
  MOCK_ADMIN_PROFILE.display_name = '演示管理员'
  MOCK_ALL_USERS.find(user => user.id === MOCK_ADMIN_USER.id)!.display_name = '演示管理员'
  MOCK_NORMAL_USER.display_name = '演示用户'
  MOCK_USER_PROFILE.display_name = '演示用户'
  MOCK_ALL_USERS.find(user => user.id === MOCK_NORMAL_USER.id)!.display_name = '演示用户'
  setMockUserToken(null)
})

describe('user management demo contracts', () => {
  beforeEach(() => {
    setMockUserToken('demo-access-token-admin')
  })

  it('supports name creation, omitted updates, clearing and current-user profile synchronization', async () => {
    const created = await handleMockRequest({
      method: 'POST', url: '/api/admin/users',
      data: JSON.stringify({ username: 'zhangsan', display_name: ' 张三 ', password: 'password123' }),
    })
    expect(created?.data).toMatchObject({ username: 'zhangsan', display_name: '张三' })
    const url = `/api/admin/users/${MOCK_ADMIN_USER.id}`
    await handleMockRequest({ method: 'PATCH', url, data: JSON.stringify({ display_name: ' 张三 ' }) })
    const omitted = await handleMockRequest({ method: 'PATCH', url, data: JSON.stringify({}) })
    expect(omitted?.data).toMatchObject({ display_name: '张三' })
    const profile = await handleMockRequest({ method: 'GET', url: '/api/users/me' })
    expect(profile?.data).toMatchObject({ display_name: '张三', username: 'Demo Admin' })
    const search = await handleMockRequest({ method: 'GET', url: '/api/admin/users', params: { search: '张三' } })
    expect(search?.data).toEqual([expect.objectContaining({ id: MOCK_ADMIN_USER.id })])
    const cleared = await handleMockRequest({ method: 'PATCH', url, data: JSON.stringify({ display_name: null }) })
    expect(cleared?.data).toMatchObject({ display_name: null, username: 'Demo Admin' })
  })

  it.each([' 张三 ', null])('synchronizes another user name (%s) before that user logs in', async displayName => {
    const adminUser = MOCK_ADMIN_USER
    const normalUser = MOCK_NORMAL_USER
    const normalProfile = MOCK_USER_PROFILE
    const expectedName = displayName?.trim() ?? null
    const url = `/api/admin/users/${normalUser.id}`
    await handleMockRequest({ method: 'PATCH', url, data: JSON.stringify({ display_name: displayName }) })
    const unchanged = await handleMockRequest({ method: 'PATCH', url, data: JSON.stringify({}) })
    expect(unchanged?.data).toMatchObject({ display_name: expectedName, username: 'Demo User' })
    expect(normalUser.display_name).toBe(expectedName)
    expect(normalProfile.display_name).toBe(expectedName)
    expect(MOCK_NORMAL_USER).toBe(normalUser)
    expect(MOCK_USER_PROFILE).toBe(normalProfile)
    expect(adminUser.display_name).toBe('演示管理员')
    expect(MOCK_ADMIN_PROFILE.display_name).toBe('演示管理员')

    await handleMockRequest({ method: 'POST', url: '/api/auth/logout' })
    await handleMockRequest({
      method: 'POST', url: '/api/auth/login',
      data: JSON.stringify({ email: 'user@demo.aether.io', password: 'demo123' }),
    })
    const profile = await handleMockRequest({ method: 'GET', url: '/api/users/me' })
    expect(profile?.data).toBe(normalProfile)
    expect(profile?.data).toMatchObject({ id: normalUser.id, display_name: expectedName, username: 'Demo User' })
  })

  it('returns a list-shaped user group response', async () => {
    const response = await handleMockRequest({
      method: 'GET',
      url: '/api/admin/user-groups',
    })

    expect(response?.data).toEqual({
      items: [],
      default_group_id: null,
    })
  })

  it('creates and lists managed keys only for the selected target user', async () => {
    const aliceId = 'demo-user-uuid-0003'
    const bobId = 'demo-user-uuid-0004'
    const created = await handleMockRequest({
      method: 'POST',
      url: `/api/admin/users/${aliceId}/api-keys`,
      data: JSON.stringify({ name: 'Alice inherited key' }),
    })

    expect(created?.data).toMatchObject({
      name: 'Alice inherited key',
      feature_settings: null,
      is_standalone: false,
    })
    expect(created?.data).toHaveProperty('key', expect.stringMatching(/^sk-ae-demo-/))

    const aliceKeys = await handleMockRequest({
      method: 'GET',
      url: `/api/admin/users/${aliceId}/api-keys`,
    })
    const bobKeys = await handleMockRequest({
      method: 'GET',
      url: `/api/admin/users/${bobId}/api-keys`,
    })

    expect(aliceKeys?.data).toMatchObject({ total: 1 })
    expect(aliceKeys?.data).toHaveProperty('api_keys', [expect.objectContaining({ name: 'Alice inherited key' })])
        expect(aliceKeys?.data).not.toHaveProperty('api_keys.0.key')
    expect(aliceKeys?.data).not.toHaveProperty('api_keys.0.fullKey')
    expect(bobKeys?.data).toEqual({ api_keys: [], total: 0 })
  })
})
