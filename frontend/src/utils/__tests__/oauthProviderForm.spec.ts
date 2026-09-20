import { describe, expect, it } from 'vitest'
import { resolveOAuthProviderDisplayName } from '../oauthProviderForm'

describe('resolveOAuthProviderDisplayName', () => {
  const saved = {
    isNew: false,
    providerType: 'custom_oidc_oa',
    formDisplayName: '',
    newDisplayName: '',
    existingDisplayName: 'ABC',
  }

  it('uses the edited name for an already saved provider', () => {
    expect(resolveOAuthProviderDisplayName({ ...saved, formDisplayName: ' 企业微信 ' })).toBe(
      '企业微信',
    )
  })

  it('keeps the stored name when the edit box is cleared', () => {
    expect(resolveOAuthProviderDisplayName({ ...saved, formDisplayName: '   ' })).toBe('ABC')
  })

  it('falls back to the provider type when nothing else is known', () => {
    expect(
      resolveOAuthProviderDisplayName({
        ...saved,
        existingDisplayName: undefined,
        templateDisplayName: undefined,
      }),
    ).toBe('custom_oidc_oa')
  })

  it('still defaults a brand new provider without a name', () => {
    expect(
      resolveOAuthProviderDisplayName({
        isNew: true,
        providerType: 'custom_oidc_work',
        formDisplayName: 'ignored',
        newDisplayName: '',
      }),
    ).toBe('Custom OIDC')
  })

  it('prefers the new provider name when the user typed one', () => {
    expect(
      resolveOAuthProviderDisplayName({
        isNew: true,
        providerType: 'custom_oidc_work',
        formDisplayName: 'ignored',
        newDisplayName: ' 企业微信 ',
      }),
    ).toBe('企业微信')
  })
})
