import { fetcher } from '@/api/api.fetch'
import type { IamPoll, IamRequestBody, IamScope } from '../types'

function root(scope: IamScope): string {
  const organisation = encodeURIComponent(scope.organisationId ?? '')
  const deployment = encodeURIComponent(scope.deploymentId ?? '')
  return `${window.apiUrl}/organisations/${organisation}/deployments/${deployment}/iam/requests`
}

async function read<T>(response: Response): Promise<T> {
  const text = await response.text()
  return (text === '' ? undefined : JSON.parse(text)) as T
}

export async function askIam(scope: IamScope, body: IamRequestBody): Promise<string> {
  const answer = await read<{ data: { request_id: string } }>(
    await fetcher('post', root(scope), { body }),
  )
  return answer.data.request_id
}

export async function fetchIamResult(scope: IamScope, requestId: string): Promise<IamPoll> {
  const response = await fetcher('get', `${root(scope)}/${encodeURIComponent(requestId)}`, {})
  if (response.status === 202) return { state: 'pending' }

  const answer = await read<{ data: { envelope: unknown } }>(response)
  return { state: 'ready', envelope: answer.data.envelope }
}
