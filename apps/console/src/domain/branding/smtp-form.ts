import type { Schemas } from '@/api/api.client'

export interface SmtpForm {
  host: string
  port: string
  encryption: Schemas.SmtpEncryption
  username: string
  password: string
  fromName: string
  fromEmail: string
}

export const ENCRYPTIONS: Schemas.SmtpEncryption[] = ['tls', 'starttls', 'none']

export const ENCRYPTION_LABELS: Record<Schemas.SmtpEncryption, string> = {
  tls: 'TLS',
  starttls: 'STARTTLS',
  none: 'None',
}

export const PORT_HINTS: Record<Schemas.SmtpEncryption, number> = {
  tls: 465,
  starttls: 587,
  none: 25,
}

const MAX_HOST = 253
const MAX_USERNAME = 255
const MAX_EMAIL = 254
const MAX_EMAIL_LOCAL = 64
const MAX_PASSWORD = 1024
const LOCAL_CHARS = /^[A-Za-z0-9!#$%&'*+/=?^_`{|}~.-]+$/
const LABEL_CHARS = /^[A-Za-z0-9-]+$/
const HOST_CHARS = /^[A-Za-z0-9._-]+$/

export function emptyForm(): SmtpForm {
  return {
    host: '',
    port: String(PORT_HINTS.starttls),
    encryption: 'starttls',
    username: '',
    password: '',
    fromName: '',
    fromEmail: '',
  }
}

export function toForm(view: Schemas.SmtpView | null): SmtpForm {
  if (!view) return emptyForm()

  return {
    host: view.host,
    port: String(view.port),
    encryption: view.encryption,
    username: view.username,
    password: '',
    fromName: view.from_name,
    fromEmail: view.from_email,
  }
}

export function isDirty(form: SmtpForm, stored: Schemas.SmtpView | null): boolean {
  const base = toForm(stored)

  return (
    form.password !== '' ||
    form.host.trim() !== base.host ||
    form.port.trim() !== base.port ||
    form.encryption !== base.encryption ||
    form.username !== base.username ||
    form.fromName.trim() !== base.fromName ||
    form.fromEmail.trim() !== base.fromEmail
  )
}

export function portHint(encryption: Schemas.SmtpEncryption): string {
  const usual = PORT_HINTS[encryption]

  return encryption === 'none'
    ? `Usually ${usual} without encryption.`
    : `Usually ${usual} with ${ENCRYPTION_LABELS[encryption]}.`
}

export function describeHostProblem(raw: string): string | null {
  const host = raw.trim()

  if (host === '') return 'Enter the address of the SMTP server, for example smtp.example.com.'
  if (host.length > MAX_HOST) return `The host holds at most ${MAX_HOST} characters.`
  if (host.includes('://') || host.includes('/')) {
    return 'The host is a name, not a URL: leave out the scheme and any path.'
  }
  if (host.includes(':')) return 'Port goes in its own field, not in the host.'
  if (!HOST_CHARS.test(host)) {
    return 'The host holds only letters, digits, hyphens, dots and underscores, with no space.'
  }
  return null
}

export function describePortProblem(raw: string): string | null {
  const port = raw.trim()

  if (port === '') return 'Enter the port of the SMTP server.'
  if (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535) {
    return 'The port is a number between 1 and 65535.'
  }
  return null
}

export function describeUsernameProblem(username: string): string | null {
  if (username.trim() === '') return 'Enter the username the server asks for.'
  if (Array.from(username).length > MAX_USERNAME) {
    return `The username holds at most ${MAX_USERNAME} characters.`
  }
  return null
}

export function describePasswordProblem(password: string, hasStoredPassword: boolean): string | null {
  if (Array.from(password).length > MAX_PASSWORD) {
    return `The password holds at most ${MAX_PASSWORD} characters.`
  }
  if (password === '' && !hasStoredPassword) return 'Enter the password the server asks for.'
  return null
}

export function describeEmailProblem(raw: string): string | null {
  const email = raw.trim()

  if (email === '') return 'Enter the address the emails are sent from.'

  const invalid = 'This is not a valid email address, for example no-reply@example.com.'
  const at = email.indexOf('@')
  if (email.length > MAX_EMAIL || /\s/.test(email) || at < 1) return invalid

  const local = email.slice(0, at)
  const domain = email.slice(at + 1)
  const localValid =
    local.length <= MAX_EMAIL_LOCAL &&
    !local.startsWith('.') &&
    !local.endsWith('.') &&
    !local.includes('..') &&
    LOCAL_CHARS.test(local)
  const domainValid =
    domain !== '' &&
    domain
      .split('.')
      .every(
        (label) =>
          LABEL_CHARS.test(label) && !label.startsWith('-') && !label.endsWith('-')
      )

  return localValid && domainValid ? null : invalid
}

export function isValid(form: SmtpForm, hasStoredPassword: boolean): boolean {
  return (
    describeHostProblem(form.host) === null &&
    describePortProblem(form.port) === null &&
    describeUsernameProblem(form.username) === null &&
    describePasswordProblem(form.password, hasStoredPassword) === null &&
    describeEmailProblem(form.fromEmail) === null
  )
}

export function toRequest(
  form: SmtpForm,
  hasStoredPassword: boolean
): Schemas.SmtpRequestBody | null {
  if (!isValid(form, hasStoredPassword)) return null

  const body: Schemas.SmtpRequestBody = {
    host: form.host.trim(),
    port: Number(form.port.trim()),
    encryption: form.encryption,
    username: form.username,
    from_email: form.fromEmail.trim(),
    from_name: form.fromName.trim(),
  }

  return form.password === '' ? body : { ...body, password: form.password }
}
