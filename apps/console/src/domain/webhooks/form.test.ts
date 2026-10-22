import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { blankHeader } from './headers'
import {
  checkEndpoint,
  checkForm,
  emptyForm,
  endpointHost,
  formFromWebhook,
  isReadyToSave,
  toRequestBody,
  type WebhookForm,
} from './form'

const valid: WebhookForm = {
  ...emptyForm('master'),
  name: 'Audit',
  endpoint: 'https://hooks.example.com/in',
  events: ['user.created'],
}

describe('the endpoint', () => {
  it('accepts https', () => {
    expect(checkEndpoint('https://hooks.example.com/in')).toBeNull()
  })

  it('refuses http and says why and what to do', () => {
    expect(checkEndpoint('http://hooks.example.com')).toMatch(/https/)
  })

  it('refuses what is not an address', () => {
    expect(checkEndpoint('hooks.example.com')).not.toBeNull()
    expect(checkEndpoint('')).not.toBeNull()
  })

  it('shows the host', () => {
    expect(endpointHost('https://hooks.example.com:8443/in?x=1')).toBe('hooks.example.com:8443')
    expect(endpointHost('not a url')).toBe('not a url')
  })
})

describe('the form', () => {
  it('has no problem when complete', () => {
    expect(checkForm(valid)).toEqual({})
    expect(isReadyToSave(valid)).toBe(true)
  })

  it('needs a name, an endpoint, a realm and one event', () => {
    const problems = checkForm({ ...emptyForm(''), name: ' ' })
    expect(Object.keys(problems).sort()).toEqual(['endpoint', 'events', 'name', 'realm'])
    expect(isReadyToSave(emptyForm(''))).toBe(false)
  })

  it('builds the request, trimming and nulling an empty description', () => {
    expect(toRequestBody({ ...valid, name: ' Audit ', description: '  ' })).toEqual({
      realm: 'master',
      name: 'Audit',
      description: null,
      endpoint: 'https://hooks.example.com/in',
      events: ['user.created'],
      headers: [],
    })
  })

  it('carries headers into the request', () => {
    const body = toRequestBody({
      ...valid,
      headers: [{ ...blankHeader('k'), name: 'x-a', value: 'v' }],
    })
    expect(body.headers).toEqual([{ name: 'x-a', value: 'v' }])
  })

  it('starts an edit from the stored webhook, headers by name only', () => {
    const webhook = {
      realm: 'master',
      name: 'Audit',
      description: null,
      endpoint: 'https://h.example.com',
      events: ['a'],
      headers: ['x-token'],
    } as unknown as Schemas.Webhook
    const form = formFromWebhook(webhook)
    expect(form.description).toBe('')
    expect(form.headers).toHaveLength(1)
    expect(form.headers[0]).toMatchObject({ name: 'x-token', stored: true, value: '' })
  })
})
