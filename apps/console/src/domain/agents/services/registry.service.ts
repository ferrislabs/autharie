import { fetcher } from '@/api/api.fetch'
import type {
  AutonomousInput,
  CredentialPoll,
  Registry,
  Scope,
  SealedCredential,
  SealingJwk,
} from '../types'

type Verb = 'get' | 'put' | 'post' | 'delete'

function root(scope: Scope): string {
  const organisation = encodeURIComponent(scope.organisationId ?? '')
  const deployment = encodeURIComponent(scope.deploymentId ?? '')
  return `${window.apiUrl}/organisations/${organisation}/deployments/${deployment}/agents`
}

async function send(verb: Verb, scope: Scope, path: string, body?: unknown): Promise<Response> {
  return fetcher(verb, `${root(scope)}${path}`, body === undefined ? {} : { body })
}

async function read<T>(response: Response): Promise<T> {
  const text = await response.text()
  return (text === '' ? undefined : JSON.parse(text)) as T
}

function client(clientId: string): string {
  return `/clients/${encodeURIComponent(clientId)}`
}

export async function getRegistry(scope: Scope): Promise<Registry> {
  return read<Registry>(await send('get', scope, '/clients'))
}

export async function setApprovalRequired(scope: Scope, approvalRequired: boolean): Promise<void> {
  await send('put', scope, '/access', { approval_required: approvalRequired })
}

export async function approveAgent(scope: Scope, clientId: string): Promise<void> {
  await send('post', scope, `${client(clientId)}/approve`)
}

export async function blockAgent(scope: Scope, clientId: string): Promise<void> {
  await send('post', scope, `${client(clientId)}/block`)
}

export async function unblockAgent(scope: Scope, clientId: string): Promise<void> {
  await send('post', scope, `${client(clientId)}/unblock`)
}

export async function deleteAgent(scope: Scope, clientId: string): Promise<void> {
  await send('delete', scope, client(clientId))
}

export async function createAutonomousAgent(
  scope: Scope,
  input: AutonomousInput,
): Promise<string> {
  const body = await read<{ data: { request_id: string } }>(
    await send('post', scope, '/autonomous', input),
  )
  return body.data.request_id
}

export async function rotateAutonomousSecret(
  scope: Scope,
  clientId: string,
  sealingKey: SealingJwk,
): Promise<string> {
  const body = await read<{ data: { request_id: string } }>(
    await send('post', scope, `/autonomous/${encodeURIComponent(clientId)}/rotate`, {
      sealing_key: sealingKey,
    }),
  )
  return body.data.request_id
}

export async function fetchCredential(scope: Scope, requestId: string): Promise<CredentialPoll> {
  const response = await send('get', scope, `/credentials/${encodeURIComponent(requestId)}`)
  if (response.status === 202) return { state: 'pending' }
  const body = await read<{ data: SealedCredential }>(response)
  return { state: 'ready', credential: body.data }
}
