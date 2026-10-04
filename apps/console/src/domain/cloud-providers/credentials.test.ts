import { describe, expect, it } from 'vitest'
import { ApiRequestError } from '@/api/api.fetch'
import {
  CREDENTIAL_IN_USE,
  EMPTY_CREDENTIAL_DRAFT,
  afterSubmit,
  canRegister,
  deletionRefusal,
  secretProblem,
  toRegisterRequest,
} from './credentials'

const SECRET = '{"access_key":"SCWXXX","secret_key":"s3cret-value","project_id":"p1"}'

describe('the secret a customer pastes', () => {
  it('accepts the three Scaleway fields', () => {
    expect(secretProblem(SECRET)).toBeNull()
  })

  it('says what is wrong rather than refusing silently', () => {
    expect(secretProblem('nope')).toBe('This is not valid JSON.')
    expect(secretProblem('[]')).toBe('Missing: access_key, secret_key, project_id.')
    expect(secretProblem('{"access_key":"a","secret_key":"b"}')).toBe('Missing: project_id.')
  })

  it('stays quiet while the field is empty', () => {
    expect(secretProblem('')).toBeNull()
  })
})

describe('adding an account', () => {
  it('needs a label and a valid secret', () => {
    expect(canRegister({ label: 'prod', provider: 'scaleway', secret: SECRET })).toBe(true)
    expect(canRegister({ label: '', provider: 'scaleway', secret: SECRET })).toBe(false)
    expect(canRegister({ label: 'prod', provider: 'scaleway', secret: 'x' })).toBe(false)
  })

  it('sends the secret as typed, with the label trimmed', () => {
    expect(toRegisterRequest({ label: ' prod ', provider: 'scaleway', secret: SECRET })).toEqual({
      label: 'prod',
      provider: 'scaleway',
      secret: SECRET,
    })
  })

  it('leaves no secret behind once submitted', () => {
    expect(afterSubmit().secret).toBe('')
    expect(afterSubmit()).toEqual(EMPTY_CREDENTIAL_DRAFT)
  })
})

describe('deleting an account', () => {
  it('shows what the platform said on a 409', () => {
    expect(deletionRefusal(new ApiRequestError(409, 'credential in use by deployment auth'))).toBe(
      'credential in use by deployment auth',
    )
  })

  it('falls back to a sentence of its own when the 409 has no message', () => {
    expect(deletionRefusal(new ApiRequestError(409, 'HTTP 409: Conflict'))).toBe(CREDENTIAL_IN_USE)
  })

  it('shows nothing when nothing failed', () => {
    expect(deletionRefusal(null)).toBeUndefined()
  })
})
