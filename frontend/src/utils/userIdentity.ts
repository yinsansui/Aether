export interface UserIdentity {
  username?: string | null
  display_name?: string | null
}

/** 用户身份文字统一显示为“姓名（登录账号）”；缺失姓名时回退到账号。 */
export function userIdentityLabel(user: UserIdentity | null | undefined): string {
  const username = user?.username?.trim() || ''
  const displayName = user?.display_name?.trim() || ''
  if (!displayName || displayName === username) return username
  return username ? `${displayName}（${username}）` : displayName
}

/** 头像只取姓名首字；身份文字请使用 userIdentityLabel。 */
export function userAvatarText(user: UserIdentity | null | undefined, length = 1): string {
  const name = user?.display_name?.trim() || user?.username?.trim() || ''
  return Array.from(name).slice(0, length).join('').toUpperCase()
}

export function normalizeDisplayName(value: string): string | null {
  return value.trim() || null
}

/** Unicode code points 与 Rust chars() 的计数一致，不使用 UTF-16 length。 */
export function displayNameError(value: string): string {
  if (/\p{Cc}/u.test(value)) return '姓名不能包含控制字符'
  if (Array.from(value.trim()).length > 100) return '姓名不能超过100个字符'
  return ''
}
