import { beforeEach, describe, expect, it, vi } from 'vitest'

const fetcher = vi.fn()

vi.mock('@/api/api.fetch', () => ({ fetcher: (...args: unknown[]) => fetcher(...args) }))

import {
  createPolicy,
  deletePolicy,
  getSettings,
  listDecisions,
  listPolicies,
  listRealms,
  updatePolicy,
  updateSettings,
} from './agents.service'

const scope = { organisationId: 'org 1', deploymentId: 'dep-1' }
const BASE = 'https://api.test/organisations/org%201/deployments/dep-1'

function answer(body: unknown, status = 200) {
  return { status, json: async () => body, text: async () => (body === undefined ? '' : JSON.stringify(body)) }
}

beforeEach(() => {
  fetcher.mockReset()
  vi.stubGlobal('window', { apiUrl: 'https://api.test' })
})

describe('the agents service', () => {
  it('reads the settings and unwraps data', async () => {
    fetcher.mockResolvedValue(answer({ data: { enabled: true, realm: 'master', organisation_mcp_enabled: true } }))
    const settings = await getSettings(scope)
    expect(fetcher).toHaveBeenCalledWith('get', `${BASE}/agents`, {})
    expect(settings.realm).toBe('master')
  })

  it('puts the settings', async () => {
    fetcher.mockResolvedValue(answer({ data: { enabled: true, realm: 'master', organisation_mcp_enabled: true } }))
    await updateSettings(scope, { enabled: true, realm: 'master' })
    expect(fetcher).toHaveBeenCalledWith('put', `${BASE}/agents`, { body: { enabled: true, realm: 'master' } })
  })

  it('lists policies', async () => {
    fetcher.mockResolvedValue(answer({ data: [{ id: 'p1' }] }))
    expect(await listPolicies(scope)).toEqual([{ id: 'p1' }])
    expect(fetcher).toHaveBeenCalledWith('get', `${BASE}/agents/policies`, {})
  })

  it('creates a policy', async () => {
    fetcher.mockResolvedValue(answer({ data: { id: 'p1' } }))
    const input = { name: 'n', enabled: true, cedar: 'permit (principal, action, resource);' }
    await createPolicy(scope, input)
    expect(fetcher).toHaveBeenCalledWith('post', `${BASE}/agents/policies`, { body: input })
  })

  it('updates a policy by id, escaping it', async () => {
    fetcher.mockResolvedValue(answer({ data: { id: 'a/b' } }))
    const input = { name: 'n', enabled: false, cedar: 'x' }
    await updatePolicy(scope, 'a/b', input)
    expect(fetcher).toHaveBeenCalledWith('put', `${BASE}/agents/policies/a%2Fb`, { body: input })
  })

  it('deletes a policy without needing a body back', async () => {
    fetcher.mockResolvedValue(answer(undefined, 204))
    await expect(deletePolicy(scope, 'p1')).resolves.toBeUndefined()
    expect(fetcher).toHaveBeenCalledWith('delete', `${BASE}/agents/policies/p1`, {})
  })

  it('passes the decision filters as the query', async () => {
    fetcher.mockResolvedValue(answer({ data: [] }))
    await listDecisions(scope, { decision: 'deny', tool: 'a/b', limit: 100 })
    expect(fetcher).toHaveBeenCalledWith('get', `${BASE}/agents/decisions`, {
      query: { decision: 'deny', tool: 'a/b', limit: 100 },
    })
  })

  it('reads the realms from the webhook endpoint', async () => {
    fetcher.mockResolvedValue(answer({ data: ['master', 'acme'] }))
    expect(await listRealms(scope)).toEqual(['master', 'acme'])
    expect(fetcher).toHaveBeenCalledWith('get', `${BASE}/webhook-realms`, {})
  })

  it('lets a refusal through', async () => {
    fetcher.mockRejectedValue(new Error('nope'))
    await expect(getSettings(scope)).rejects.toThrow('nope')
  })
})
