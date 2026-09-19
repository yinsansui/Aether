import type { OAuthProviderTestRequest, OAuthProviderTestResponse } from '@/api/oauth'

export interface OAuthConfigTestFormInput {
  client_id: string
  client_secret: string
  authorization_url_override: string
  token_url_override: string
  redirect_uri: string
  extra_config_json: string
}

export function parseJsonOrNull(input: string): Record<string, unknown> | null {
  const raw = input.trim()
  if (!raw) return null
  return JSON.parse(raw)
}

/**
 * 「测试」按钮提交的 payload。extra_config 必须一起带上：未保存的自定义
 * provider 只能从这里解析 allowed_domains，漏传会把可达的端点误判为不可达。
 */
export function buildOAuthConfigTestPayload(
  form: OAuthConfigTestFormInput,
): OAuthProviderTestRequest {
  return {
    client_id: form.client_id.trim(),
    client_secret: form.client_secret.trim() || undefined,
    authorization_url_override: form.authorization_url_override.trim() || null,
    token_url_override: form.token_url_override.trim() || null,
    redirect_uri: form.redirect_uri.trim(),
    extra_config: parseJsonOrNull(form.extra_config_json),
  }
}

export type OAuthConfigTestSeverity = 'success' | 'warning' | 'error'

export interface OAuthConfigTestSummary {
  severity: OAuthConfigTestSeverity
  message: string
  failures: string[]
  warnings: string[]
}

function describeSecretStatus(status: string | undefined): string | null {
  const normalized = (status || '').trim().toLowerCase()
  if (!normalized || normalized === 'likely_valid' || normalized === 'configured') return null
  if (normalized === 'invalid') return 'Secret 无效'
  if (normalized === 'unsupported') return 'Secret 不受支持'
  if (normalized === 'not_provided') return 'Secret 未提供'
  if (normalized === 'unknown') return 'Secret 未验证'
  return `Secret: ${status}`
}

export function summarizeOAuthConfigTest(result: OAuthProviderTestResponse): OAuthConfigTestSummary {
  const failures: string[] = []
  const warnings: string[] = []

  if (!result.authorization_url_reachable) {
    failures.push('Authorization URL 不可达')
  }
  if (!result.token_url_reachable) {
    failures.push('Token URL 不可达')
  }

  const secretStatus = (result.secret_status || '').trim().toLowerCase()
  const secretMessage = describeSecretStatus(result.secret_status)
  if (secretMessage && (secretStatus === 'invalid' || secretStatus === 'unsupported')) {
    failures.push(secretMessage)
  } else if (secretMessage) {
    warnings.push(secretMessage)
  }

  if (failures.length > 0) {
    return {
      severity: 'error',
      message: `测试失败：${failures.join('，')}`,
      failures,
      warnings,
    }
  }

  if (warnings.length > 0) {
    return {
      severity: 'warning',
      message: `测试完成，但有未确认项：${warnings.join('，')}`,
      failures,
      warnings,
    }
  }

  return {
    severity: 'success',
    message: '测试通过',
    failures,
    warnings,
  }
}
