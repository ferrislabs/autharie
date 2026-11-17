import { describe, expect, it } from 'vitest'
import { canIssueAdminAccess, curlCommands, parseAdminAccess, tokenUrlOf } from './admin-access'

const EP = 'https://acme.example.com'

describe('parseAdminAccess', () => {
  it('reads both fields', () => {
    expect(parseAdminAccess({ client_id: 'autharie-admin', client_secret: 's' })).toEqual({
      clientId: 'autharie-admin',
      clientSecret: 's',
    })
  })

  it.each([null, 'x', {}, { client_id: 'a' }, { client_id: '', client_secret: 's' }, { client_id: 'a', client_secret: 2 }])(
    'rejects %j',
    (value) => {
      expect(() => parseAdminAccess(value)).toThrow()
    },
  )
})

describe('admin access commands', () => {
  it('targets the master realm token endpoint', () => {
    expect(tokenUrlOf(EP)).toBe(`${EP}/api/realms/master/protocol/openid-connect/token`)
  })

  it('uses placeholders and the verified admin routes', () => {
    const text = curlCommands(EP)
    expect(text).toContain('$CLIENT_SECRET')
    expect(text).toContain('$EMAIL')
    expect(text).toContain('$PASSWORD')
    expect(text).toContain('client_id=autharie-admin')
    expect(text).toContain(`${EP}/api/realms/master/users'`)
    expect(text).toContain('/reset-password')
    expect(text).toContain('temporary: false')
    expect(text).toContain('/roles/$ROLE_ID')
    expect(text).toContain('roles?qualified_name=autharie-admin&scope=realm')
  })
})

describe('canIssueAdminAccess', () => {
  const ok = { kind: 'ferriskey', running: true, mayManage: true, endpoint: EP }

  it('needs ferriskey, running, permission and an endpoint', () => {
    expect(canIssueAdminAccess(ok)).toBe(true)
    expect(canIssueAdminAccess({ ...ok, kind: 'keycloak' })).toBe(false)
    expect(canIssueAdminAccess({ ...ok, running: false })).toBe(false)
    expect(canIssueAdminAccess({ ...ok, mayManage: false })).toBe(false)
    expect(canIssueAdminAccess({ ...ok, endpoint: undefined })).toBe(false)
  })
})
