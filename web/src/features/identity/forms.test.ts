import { describe, expect, it } from 'vitest'

import { directoryDraft, directoryPatch, needsPassword, newDirectoryBody } from './ldap-form'
import { EMPTY_PROVIDER, newProviderBody, providerDraft, providerPatch } from './oidc-form'
import { directoryView, providerView } from './test-data'

describe('OIDC provider form', () => {
  it('sends a secret only when one was typed, and null to make a public client', () => {
    const draft = providerDraft(providerView(1, 'corp'))

    expect(providerPatch(draft, [])).toEqual({})
    expect(providerPatch({ ...draft, client_secret: 'new' }, ['client_secret'])).toEqual({
      client_secret: 'new',
    })
    expect(providerPatch({ ...draft, clear_secret: true }, ['clear_secret'])).toEqual({
      client_secret: null,
    })
  })

  it('sends blank optional fields as null', () => {
    const draft = { ...providerDraft(providerView(1, 'corp')), admin_group: '  ' }

    expect(providerPatch(draft, ['admin_group'])).toEqual({ admin_group: null })
    expect(newProviderBody({ ...EMPTY_PROVIDER, name: ' okta ' })).toMatchObject({
      name: 'okta',
      client_secret: null,
      group_claim: null,
    })
  })
})

describe('LDAP directory form', () => {
  it('never prefills the bind password and patches only edited fields', () => {
    const draft = directoryDraft(directoryView())

    expect(draft.bind_password).toBe('')
    expect(directoryPatch({ ...draft, name: 'Corp' }, ['name'])).toEqual({ name: 'Corp' })
    expect(directoryPatch({ ...draft, group_base_dn: '' }, ['group_base_dn'])).toEqual({
      group_base_dn: null,
    })
  })

  it('needs the password again when the connection moves', () => {
    const draft = directoryDraft(directoryView())

    expect(needsPassword({ ...draft, url: 'ldaps://x' }, ['url'])).toBe(true)
    expect(needsPassword({ ...draft, tls_verify: false }, ['tls_verify'])).toBe(true)
    expect(needsPassword({ ...draft, url: 'ldaps://x', bind_password: 'p' }, ['url'])).toBe(false)
    expect(needsPassword({ ...draft, name: 'n' }, ['name'])).toBe(false)
    expect(needsPassword({ ...draft, url: 'ldaps://x', bind_dn: '' }, ['url', 'bind_dn'])).toBe(
      false,
    )
  })

  it('tests and creates from the whole form', () => {
    const body = newDirectoryBody({ ...directoryDraft(directoryView()), bind_password: 'p' })

    expect(body).toMatchObject({ url: 'ldaps://ldap.example.org', bind_password: 'p' })
  })
})
