import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import {
  describeEmailProblem,
  describeHostProblem,
  describePasswordProblem,
  describePortProblem,
  describeUsernameProblem,
  emptyForm,
  isDirty,
  isValid,
  portHint,
  toForm,
  toRequest,
  type SmtpForm,
} from './smtp-form'

const view: Schemas.SmtpView = {
  host: 'smtp.example.com',
  port: 465,
  encryption: 'tls',
  username: 'mailer',
  from_email: 'no-reply@example.com',
  from_name: 'Acme',
  password_set: true,
  state: { phase: 'Applied' },
  updated_at: '2026-01-01T00:00:00Z',
}

const filled: SmtpForm = {
  host: 'smtp.example.com',
  port: '465',
  encryption: 'tls',
  username: 'mailer',
  password: 'hunter2',
  fromName: 'Acme',
  fromEmail: 'no-reply@example.com',
}

describe('the form model', () => {
  it('starts empty on STARTTLS and its usual port', () => {
    expect(emptyForm()).toEqual({
      host: '',
      port: '587',
      encryption: 'starttls',
      username: '',
      password: '',
      fromName: '',
      fromEmail: '',
    })
  })

  it('reads a stored provider without its password', () => {
    expect(toForm(view)).toEqual({ ...filled, password: '' })
  })

  it('reads no provider as the empty form', () => {
    expect(toForm(null)).toEqual(emptyForm())
  })
})

describe('what counts as a change', () => {
  it('is not a change to leave the stored values alone', () => {
    expect(isDirty(toForm(view), view)).toBe(false)
    expect(isDirty(emptyForm(), null)).toBe(false)
  })

  it('is a change to edit any field', () => {
    const form = toForm(view)

    for (const edit of [
      { host: 'mail.example.com' },
      { port: '587' },
      { encryption: 'starttls' as const },
      { username: 'other' },
      { fromName: 'Other' },
      { fromEmail: 'hello@example.com' },
    ]) {
      expect(isDirty({ ...form, ...edit }, view), JSON.stringify(edit)).toBe(true)
    }
  })

  it('is a change to type a new password', () => {
    expect(isDirty({ ...toForm(view), password: 'new' }, view)).toBe(true)
  })

  it('ignores the spaces around the host and the addresses', () => {
    const form = { ...toForm(view), host: ' smtp.example.com ', fromEmail: ' no-reply@example.com ' }

    expect(isDirty(form, view)).toBe(false)
  })

  it('is a change to start filling in the first provider', () => {
    expect(isDirty({ ...emptyForm(), host: 'smtp.example.com' }, null)).toBe(true)
  })
})

describe('the host', () => {
  it.each(['smtp.example.com', '10.0.0.7', 'mail_relay.internal', 'a-b.example.com'])(
    'accepts %s',
    (host) => expect(describeHostProblem(host)).toBeNull()
  )

  it('asks for one when empty', () => {
    expect(describeHostProblem('  ')).toMatch(/Enter/)
  })

  it('refuses a scheme, a path and a space', () => {
    expect(describeHostProblem('smtp://mail.example.com')).toMatch(/not a URL/)
    expect(describeHostProblem('mail.example.com/relay')).toMatch(/not a URL/)
    expect(describeHostProblem('mail example.com')).toMatch(/letters, digits/)
  })

  it('refuses a port stuck to the name', () => {
    expect(describeHostProblem('mail.example.com:587')).toMatch(/Port/)
  })

  it('refuses a name over 253 characters', () => {
    expect(describeHostProblem('a'.repeat(253))).toBeNull()
    expect(describeHostProblem('a'.repeat(254))).toMatch(/253/)
  })
})

describe('the port', () => {
  it.each(['1', '25', '587', '65535'])('accepts %s', (port) =>
    expect(describePortProblem(port)).toBeNull()
  )

  it.each(['', '0', '65536', '-1', '5.5', 'abc', '58 7'])('refuses "%s"', (port) =>
    expect(describePortProblem(port)).toMatch(/1 and 65535|Enter/)
  )
})

describe('the username', () => {
  it('is required', () => {
    expect(describeUsernameProblem('')).toMatch(/Enter/)
    expect(describeUsernameProblem('   ')).toMatch(/Enter/)
    expect(describeUsernameProblem('apikey')).toBeNull()
  })

  it('is at most 255 characters', () => {
    expect(describeUsernameProblem('u'.repeat(255))).toBeNull()
    expect(describeUsernameProblem('u'.repeat(256))).toMatch(/255/)
  })
})

describe('the from address', () => {
  it.each(['no-reply@example.com', 'a.b+tag@mail.example.co', 'x@localhost'])(
    'accepts %s',
    (email) => expect(describeEmailProblem(email)).toBeNull()
  )

  it('asks for one when empty', () => {
    expect(describeEmailProblem('')).toMatch(/Enter/)
  })

  it.each([
    'plain',
    '@example.com',
    'a@',
    'a b@example.com',
    '.a@example.com',
    'a.@example.com',
    'a..b@example.com',
    'a@-example.com',
    'a@example..com',
    'a@@example.com',
  ])('refuses %s', (email) => expect(describeEmailProblem(email)).toMatch(/valid email/))
})

describe('the password', () => {
  it('is required for the first save', () => {
    expect(describePasswordProblem('', false)).toMatch(/Enter/)
    expect(describePasswordProblem('secret', false)).toBeNull()
  })

  it('may be left empty to keep the stored one', () => {
    expect(describePasswordProblem('', true)).toBeNull()
  })

  it('is at most 1024 characters', () => {
    expect(describePasswordProblem('p'.repeat(1024), true)).toBeNull()
    expect(describePasswordProblem('p'.repeat(1025), true)).toMatch(/1024/)
  })
})

describe('whether the form can be saved', () => {
  it('accepts a complete form', () => {
    expect(isValid(filled, false)).toBe(true)
  })

  it('does not accept the first save without a password', () => {
    expect(isValid({ ...filled, password: '' }, false)).toBe(false)
  })

  it('accepts a later save without one', () => {
    expect(isValid({ ...filled, password: '' }, true)).toBe(true)
  })

  it('does not accept a single bad field', () => {
    expect(isValid({ ...filled, host: 'smtp://x' }, true)).toBe(false)
    expect(isValid({ ...filled, port: '0' }, true)).toBe(false)
    expect(isValid({ ...filled, username: '' }, true)).toBe(false)
    expect(isValid({ ...filled, fromEmail: 'nope' }, true)).toBe(false)
  })

  it('does not need a from name', () => {
    expect(isValid({ ...filled, fromName: '' }, true)).toBe(true)
  })
})

describe('the request', () => {
  it('sends the typed values, trimmed, with the port as a number', () => {
    expect(
      toRequest({ ...filled, host: ' smtp.example.com ', fromName: ' Acme ' }, false)
    ).toEqual({
      host: 'smtp.example.com',
      port: 465,
      encryption: 'tls',
      username: 'mailer',
      password: 'hunter2',
      from_email: 'no-reply@example.com',
      from_name: 'Acme',
    })
  })

  it('leaves the password out when none is typed and one is stored', () => {
    const body = toRequest({ ...filled, password: '' }, true)

    expect(body).not.toBeNull()
    expect(body).not.toHaveProperty('password')
  })

  it('sends a new password over the stored one', () => {
    expect(toRequest(filled, true)?.password).toBe('hunter2')
  })

  it('keeps the spaces inside a password', () => {
    expect(toRequest({ ...filled, password: ' a b ' }, true)?.password).toBe(' a b ')
  })

  it('makes no request without a password when none is stored', () => {
    expect(toRequest({ ...filled, password: '' }, false)).toBeNull()
  })

  it('makes no request from an invalid form', () => {
    expect(toRequest({ ...filled, port: '99999' }, true)).toBeNull()
  })
})

describe('the port hint', () => {
  it('names the usual port of each encryption', () => {
    expect(portHint('tls')).toBe('Usually 465 with TLS.')
    expect(portHint('starttls')).toBe('Usually 587 with STARTTLS.')
    expect(portHint('none')).toBe('Usually 25 without encryption.')
  })
})
