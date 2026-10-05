import { describe, expect, it } from 'vitest'
import { ApiRequestError } from '@/api/api.fetch'
import {
  CREDENTIAL_IN_USE,
  afterSubmit,
  canRegister,
  deletionRefusal,
  emptyDraft,
  toRegisterRequest,
} from './credentials'
import { PROVIDERS, providerById } from './providers'

const FILLED = {
  provider: 'scaleway' as const,
  values: {
    name: ' prod ',
    access_key: ' SCWXXX ',
    secret_key: 's3cret-value',
    organization_id: 'org-1',
    project_id: 'p1',
  },
}

describe('the providers a credential can be created for', () => {
  it('lists Scaleway only', () => {
    expect(PROVIDERS.map((provider) => provider.id)).toEqual(['scaleway'])
    expect(PROVIDERS[0].label).toBe('Scaleway')
  })

  it('asks for the five Scaleway fields, the secret one masked', () => {
    const fields = providerById('scaleway').fields
    expect(fields.map((field) => field.label)).toEqual([
      'Name',
      'Access key',
      'Secret access key',
      'Organization id',
      'Project id',
    ])
    expect(fields.filter((field) => field.masked).map((field) => field.id)).toEqual(['secret_key'])
  })
})

describe('adding an account', () => {
  it('needs all five fields', () => {
    expect(canRegister(FILLED)).toBe(true)
    for (const id of Object.keys(FILLED.values)) {
      expect(canRegister({ ...FILLED, values: { ...FILLED.values, [id]: '' } })).toBe(false)
      expect(canRegister({ ...FILLED, values: { ...FILLED.values, [id]: '   ' } })).toBe(false)
    }
    expect(canRegister(emptyDraft('scaleway'))).toBe(false)
  })

  it('builds the secret JSON itself, with the label trimmed', () => {
    expect(toRegisterRequest(FILLED)).toEqual({
      provider: 'scaleway',
      label: 'prod',
      secret:
        '{"access_key":"SCWXXX","secret_key":"s3cret-value","project_id":"p1","organization_id":"org-1"}',
    })
  })

  it('leaves no secret behind once submitted, and keeps the rest', () => {
    const after = afterSubmit(FILLED)
    expect(after.values.secret_key).toBeUndefined()
    expect(JSON.stringify(after)).not.toContain('s3cret-value')
    expect(after.values.name).toBe(' prod ')
    expect(after.values.project_id).toBe('p1')
    expect(canRegister(after)).toBe(false)
  })
})

describe('deleting an account', () => {
  it('shows what the platform said on a 409', () => {
    expect(deletionRefusal(new ApiRequestError(409, 'credential in use by deployment auth'))).toBe(
      'credential in use by deployment auth'
    )
  })

  it('falls back to a sentence of its own when the 409 has no message', () => {
    expect(deletionRefusal(new ApiRequestError(409, 'HTTP 409: Conflict'))).toBe(CREDENTIAL_IN_USE)
  })

  it('shows nothing when nothing failed', () => {
    expect(deletionRefusal(null)).toBeUndefined()
  })
})
