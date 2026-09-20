const NEW_PROVIDER_DEFAULT_DISPLAY_NAME = 'Custom OIDC'

export interface OAuthProviderDisplayNameInput {
  isNew: boolean
  providerType: string
  /** 表单里已保存 provider 的显示名称输入框 */
  formDisplayName: string
  /** 「新建配置」里的显示名称输入框 */
  newDisplayName: string
  existingDisplayName?: string
  templateDisplayName?: string
}

/**
 * 解析保存时提交的显示名称。已保存的 provider 允许直接改名，输入框留空时
 * 依次回落到已保存值、模板名和配置标识，保证提交给后端的名称非空。
 */
export function resolveOAuthProviderDisplayName(input: OAuthProviderDisplayNameInput): string {
  if (input.isNew) {
    return input.newDisplayName.trim() || NEW_PROVIDER_DEFAULT_DISPLAY_NAME
  }
  return (
    input.formDisplayName.trim()
    || input.existingDisplayName
    || input.templateDisplayName
    || input.providerType
  )
}
