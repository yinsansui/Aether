import { describe, expect, it } from 'vitest'
import { displayNameError, normalizeDisplayName, userAvatarText, userIdentityLabel } from '../userIdentity'

describe('user identity', () => {
  it('displays trimmed Chinese names but preserves login accounts in target labels', () => {
    const user = { username: 'zhangsan', display_name: ' 张三 ' }
    expect(userIdentityLabel(user)).toBe('张三（zhangsan）')
    expect(userAvatarText(user)).toBe('张')
    expect(userAvatarText({ username: 'demo', display_name: '𠮷野' })).toBe('𠮷')
  })

  it.each([undefined, null, '', '   '])('falls back for absent or blank names (%s)', display_name => {
    expect(userIdentityLabel({ username: 'login', display_name })).toBe('login')
  })

  it('handles missing accounts and avoids repeating identical names', () => {
    expect(userIdentityLabel(null)).toBe('')
    expect(userIdentityLabel({ display_name: ' 张三 ' })).toBe('张三')
    expect(userIdentityLabel({ username: null, display_name: '张三' })).toBe('张三')
    expect(userIdentityLabel({ username: ' zhangsan ', display_name: ' zhangsan ' })).toBe('zhangsan')
    expect(userIdentityLabel({ username: ' zhangsan ', display_name: ' 张三 ' })).toBe('张三（zhangsan）')
    expect(userIdentityLabel({ username: '   ', display_name: '   ' })).toBe('')
  })

  it('counts Unicode characters rather than UTF-16 units and rejects controls', () => {
    expect(displayNameError('𠮷'.repeat(100))).toBe('')
    expect(displayNameError('𠮷'.repeat(101))).toBe('姓名不能超过100个字符')
    expect(displayNameError(' 玛丽·Jane Smith ')).toBe('')
    expect(displayNameError('张\n三')).toBe('姓名不能包含控制字符')
    expect(displayNameError('\u0085')).toBe('姓名不能包含控制字符')
    expect(normalizeDisplayName(' 张三 ')).toBe('张三')
    expect(normalizeDisplayName('   ')).toBeNull()
  })
})
