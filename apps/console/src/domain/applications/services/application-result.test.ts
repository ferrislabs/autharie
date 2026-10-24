import { describe, expect, it } from 'vitest'
import fixture from '../../../../../../libs/autharie-operator-core/tests/fixtures/iam-envelope.json'
import { parseApplicationDetail, parseApplicationList } from './application-result'

const SUMMARY = {
  id: '0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a22',
  client_id: 'web',
  name: 'Web',
  enabled: true,
  client_type: 'confidential',
  public_client: false,
  service_account_enabled: false,
  protocol: 'openid-connect',
  registration_source: 'admin',
  created_at: '2026-10-23T10:00:00Z',
}

const DETAIL = {
  ...SUMMARY,
  direct_access_grants_enabled: false,
  require_pkce: true,
  token_exchange_enabled: false,
  consent_required: false,
  redirect_uris: ['https://app.example/cb'],
  access_token_lifetime: 300,
  updated_at: '2026-10-24T10:00:00Z',
}

describe('reading a list answer', () => {
  it('keeps the items and the next page', () => {
    expect(parseApplicationList({ items: [SUMMARY], next_page: 2 })).toEqual({
      items: [SUMMARY],
      next_page: 2,
    })
  })

  it('has no next page when there is none, or when it is null', () => {
    expect(parseApplicationList({ items: [] })).toEqual({ items: [] })
    expect(parseApplicationList({ items: [], next_page: null })).toEqual({ items: [] })
  })

  it('reads what the Rust side wrote', () => {
    const list = parseApplicationList(fixture.plaintext)

    expect(list.items).toHaveLength(fixture.plaintext.items.length)
    expect(list.next_page).toBe(2)
  })

  it('carries nothing it does not know, so a stray secret is never held', () => {
    const list = parseApplicationList({
      items: [{ ...SUMMARY, secret: 'hunter2' }],
      secret: 'hunter2',
    })

    expect(JSON.stringify(list)).not.toContain('hunter2')
    expect(Object.keys(list.items[0]).sort()).toEqual(Object.keys(SUMMARY).sort())
  })

  it.each([
    ['not an object', 'x'],
    ['items missing', {}],
    ['items not a list', { items: {} }],
    ['an item missing a field', { items: [{ ...SUMMARY, name: undefined }] }],
    ['a flag that is not a boolean', { items: [{ ...SUMMARY, enabled: 'yes' }] }],
    ['a next page that is not a number', { items: [], next_page: '2' }],
  ])('refuses %s', (_name, value) => {
    expect(() => parseApplicationList(value)).toThrow(/shape/)
  })
})

describe('reading a detail answer', () => {
  it('keeps every field of the contract', () => {
    expect(parseApplicationDetail(DETAIL)).toEqual(DETAIL)
  })

  it('leaves the lifetimes out when there are none', () => {
    const detail = parseApplicationDetail({ ...DETAIL, access_token_lifetime: undefined })

    expect(detail).not.toHaveProperty('access_token_lifetime')
    expect(detail).not.toHaveProperty('refresh_token_lifetime')
  })

  it('carries nothing it does not know, so a stray secret is never held', () => {
    const detail = parseApplicationDetail({ ...DETAIL, secret: 'hunter2' })

    expect(JSON.stringify(detail)).not.toContain('hunter2')
    expect(detail).not.toHaveProperty('secret')
  })

  it.each([
    ['redirect_uris missing', { ...DETAIL, redirect_uris: undefined }],
    ['redirect_uris with a non string', { ...DETAIL, redirect_uris: [1] }],
    ['updated_at missing', { ...DETAIL, updated_at: undefined }],
    ['a lifetime that is not a number', { ...DETAIL, id_token_lifetime: 'long' }],
    ['a list answer', { items: [] }],
  ])('refuses %s', (_name, value) => {
    expect(() => parseApplicationDetail(value)).toThrow(/shape/)
  })
})
