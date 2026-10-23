import { beforeEach, describe, expect, it, vi } from 'vitest'

const fetcher = vi.fn()

vi.mock('@/api/api.fetch', () => ({ fetcher: (...args: unknown[]) => fetcher(...args) }))

import {
  approveAgent,
  blockAgent,
  createAutonomousAgent,
  deleteAgent,
  fetchCredential,
  getRegistry,
  rotateAutonomousSecret,
  setApprovalRequired,
  unblockAgent,
} from './registry.service'

const scope = { organisationId: 'org 1', deploymentId: 'dep-1' }
const BASE = 'https://api.test/organisations/org%201/deployments/dep-1/agents'
const KEY = { kty: 'RSA', n: 'n', e: 'AQAB', alg: 'RSA-OAEP-256' } as const

function answer(body: unknown, status = 200) {
  return { status, text: async () => (body === undefined ? '' : JSON.stringify(body)) }
}

beforeEach(() => {
  fetcher.mockReset()
  vi.stubGlobal('window', { apiUrl: 'https://api.test' })
})

describe('the registry service', () => {
  it('reads the registry whole', async () => {
    const body = { approval_required: true, data: [{ client_id: 'a' }], truncated: false }
    fetcher.mockResolvedValue(answer(body))

    expect(await getRegistry(scope)).toEqual(body)
    expect(fetcher).toHaveBeenCalledWith('get', `${BASE}/clients`, {})
  })

  it('puts the approval mode', async () => {
    fetcher.mockResolvedValue(answer({ data: { approval_required: true } }))

    await setApprovalRequired(scope, true)

    expect(fetcher).toHaveBeenCalledWith('put', `${BASE}/access`, {
      body: { approval_required: true },
    })
  })

  it('approves, blocks, unblocks and deletes by escaped client id', async () => {
    fetcher.mockResolvedValue(answer(undefined, 204))
    const id = 'https://app.example/c.json'
    const escaped = encodeURIComponent(id)

    await approveAgent(scope, id)
    await blockAgent(scope, id)
    await unblockAgent(scope, id)
    await deleteAgent(scope, id)

    expect(fetcher.mock.calls).toEqual([
      ['post', `${BASE}/clients/${escaped}/approve`, {}],
      ['post', `${BASE}/clients/${escaped}/block`, {}],
      ['post', `${BASE}/clients/${escaped}/unblock`, {}],
      ['delete', `${BASE}/clients/${escaped}`, {}],
    ])
  })

  it('creates an autonomous agent with the public key and returns the request id', async () => {
    fetcher.mockResolvedValue(answer({ data: { request_id: 'r1' } }, 202))
    const input = { name: 'n8n', sealing_key: KEY }

    expect(await createAutonomousAgent(scope, input)).toBe('r1')
    expect(fetcher).toHaveBeenCalledWith('post', `${BASE}/autonomous`, { body: input })
  })

  it('rotates a secret with a new public key', async () => {
    fetcher.mockResolvedValue(answer({ data: { request_id: 'r2' } }, 202))

    expect(await rotateAutonomousSecret(scope, 'agent-x', KEY)).toBe('r2')
    expect(fetcher).toHaveBeenCalledWith('post', `${BASE}/autonomous/agent-x/rotate`, {
      body: { sealing_key: KEY },
    })
  })

  it('tells a pending credential from a ready one by the status', async () => {
    fetcher.mockResolvedValueOnce(answer({ data: { status: 'pending' } }, 202))
    fetcher.mockResolvedValueOnce(
      answer(
        { data: { client_id: 'agent-x', sealed: 's', token_url: 't', audience: 'a' } },
        200,
      ),
    )

    expect(await fetchCredential(scope, 'r1')).toEqual({ state: 'pending' })
    expect(await fetchCredential(scope, 'r1')).toEqual({
      state: 'ready',
      credential: { client_id: 'agent-x', sealed: 's', token_url: 't', audience: 'a' },
    })
    expect(fetcher).toHaveBeenCalledWith('get', `${BASE}/credentials/r1`, {})
  })
})
