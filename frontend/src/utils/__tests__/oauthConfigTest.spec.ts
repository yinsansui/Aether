import { describe, expect, it } from 'vitest'
import {
  buildOAuthConfigTestPayload,
  parseJsonOrNull,
  summarizeOAuthConfigTest,
} from '../oauthConfigTest'

describe('summarizeOAuthConfigTest', () => {
  it('marks unreachable endpoints and unsupported secret as failed', () => {
    const summary = summarizeOAuthConfigTest({
      authorization_url_reachable: false,
      token_url_reachable: false,
      secret_status: 'unsupported',
      details: 'OAuth 配置测试仅支持 Rust execution runtime',
    })

    expect(summary.severity).toBe('error')
    expect(summary.failures).toEqual([
      'Authorization URL 不可达',
      'Token URL 不可达',
      'Secret 不受支持',
    ])
    expect(summary.message).toBe('测试失败：Authorization URL 不可达，Token URL 不可达，Secret 不受支持')
  })

  it('uses warning when only secret validation is inconclusive', () => {
    const summary = summarizeOAuthConfigTest({
      authorization_url_reachable: true,
      token_url_reachable: true,
      secret_status: 'unknown',
    })

    expect(summary.severity).toBe('warning')
    expect(summary.warnings).toEqual(['Secret 未验证'])
  })

  it('marks fully reachable config with a likely valid secret as successful', () => {
    const summary = summarizeOAuthConfigTest({
      authorization_url_reachable: true,
      token_url_reachable: true,
      secret_status: 'likely_valid',
    })

    expect(summary.severity).toBe('success')
    expect(summary.message).toBe('测试通过')
  })

  it('accepts a configured secret because OAuth secrets are verified during code exchange', () => {
    const summary = summarizeOAuthConfigTest({
      authorization_url_reachable: true,
      token_url_reachable: true,
      secret_status: 'configured',
    })

    expect(summary.severity).toBe('success')
  })
})

describe('buildOAuthConfigTestPayload', () => {
  const form = {
    client_id: ' aether ',
    client_secret: '',
    authorization_url_override: 'https://oa.abcyun.cn/api/management/login/oidc/authorize',
    token_url_override: 'https://oa.abcyun.cn/api/management/login/oidc/token',
    redirect_uri: ' http://172.19.10.176:8084/api/oauth/custom_oidc_oa/callback ',
    extra_config_json: '{"allowed_domains":["oa.abcyun.cn"]}',
  }

  // 未保存的自定义 provider 只能从 extra_config 解析 allowed_domains，
  // 漏传会让后端把可达的端点误判为不可达。
  it('sends extra_config so an unsaved provider can resolve its allowed domains', () => {
    const payload = buildOAuthConfigTestPayload(form)

    expect(payload.extra_config).toEqual({ allowed_domains: ['oa.abcyun.cn'] })
    expect(payload.client_id).toBe('aether')
    expect(payload.redirect_uri).toBe(
      'http://172.19.10.176:8084/api/oauth/custom_oidc_oa/callback',
    )
  })

  it('omits an empty secret and mirrors blank overrides as null', () => {
    const payload = buildOAuthConfigTestPayload({ ...form, authorization_url_override: '' })

    expect(payload.client_secret).toBeUndefined()
    expect(payload.authorization_url_override).toBeNull()
  })

  it('does not send a userinfo override, which the test endpoint ignores', () => {
    const payload = buildOAuthConfigTestPayload(form)

    expect(payload).not.toHaveProperty('userinfo_url_override')
  })
})

describe('parseJsonOrNull', () => {
  it('treats a blank input as absent config instead of an empty object', () => {
    expect(parseJsonOrNull('   ')).toBeNull()
  })

  it('parses a JSON object body', () => {
    expect(parseJsonOrNull('{"allowed_domains":["oa.abcyun.cn"]}')).toEqual({
      allowed_domains: ['oa.abcyun.cn'],
    })
  })
})
