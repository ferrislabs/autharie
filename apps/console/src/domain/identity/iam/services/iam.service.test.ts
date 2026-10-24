import { beforeEach, describe, expect, it, vi } from 'vitest'

const fetcher = vi.fn()

vi.mock('@/api/api.fetch', () => ({ fetcher: (...args: unknown[]) => fetcher(...args) }))

import { askIam, fetchIamResult } from './iam.service'

const scope = { organisationId: 'org 1', deploymentId: 'dep-1' }
const BASE = 'https://api.test/organisations/org%201/deployments/dep-1/iam/requests'
const KEY = { kty: 'RSA', n: 'n', e: 'AQAB', alg: 'RSA-OAEP-256' } as const

function answer(body: unknown, status = 200) {
  return { status, text: async () => (body === undefined ? '' : JSON.stringify(body)) }
}

beforeEach(() => {
  fetcher.mockReset()
  vi.stubGlobal('window', { apiUrl: 'https://api.test' })
})

describe('the IAM service', () => {
  it('posts the operation with the public key only and returns the request id', async () => {
    fetcher.mockResolvedValue(answer({ data: { request_id: 'r1' } }, 202))
    const body = {
      realm: 'master',
      operation: { type: 'applications.list', page: 2, limit: 25, search: 'web' },
      sealing_key: KEY,
    } as const

    expect(await askIam(scope, body)).toBe('r1')
    expect(fetcher).toHaveBeenCalledWith('post', BASE, { body })
  })

  it('reads the pending state from a 202', async () => {
    fetcher.mockResolvedValue(answer({ data: { status: 'pending' } }, 202))

    expect(await fetchIamResult(scope, 'r1')).toEqual({ state: 'pending' })
    expect(fetcher).toHaveBeenCalledWith('get', `${BASE}/r1`, {})
  })

  it('hands back the envelope of a 200 untouched', async () => {
    const envelope = { v: 1, alg: 'RSA-OAEP-256+A256GCM', ek: 'a', iv: 'b', ct: 'c' }
    fetcher.mockResolvedValue(answer({ data: { envelope } }, 200))

    expect(await fetchIamResult(scope, 'r1')).toEqual({ state: 'ready', envelope })
  })

  it('escapes the request id in the address', async () => {
    fetcher.mockResolvedValue(answer(undefined, 202))

    await fetchIamResult(scope, 'a/b')

    expect(fetcher).toHaveBeenCalledWith('get', `${BASE}/a%2Fb`, {})
  })

  it('lets a refusal through for the flow to read its status', async () => {
    const refused = Object.assign(new Error('gone'), { status: 410 })
    fetcher.mockRejectedValue(refused)

    await expect(fetchIamResult(scope, 'r1')).rejects.toBe(refused)
  })
})
