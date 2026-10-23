import { describe, expect, it } from 'vitest'
import { slugFromName, validateName, validateServerUrl, validateSlug } from './server-rules'

describe('the URL of a customer server', () => {
  it('accepts a public https URL', () => {
    for (const url of [
      'https://tools.example.com/mcp',
      'https://tools.example.com:8443/mcp?tenant=a',
      'https://8.8.8.8/mcp',
      'https://[2606:4700:4700::1111]/mcp',
      ' https://tools.example.com/mcp ',
    ]) {
      expect(validateServerUrl(url), url).toBeNull()
    }
  })

  it('refuses plain http, credentials and fragments', () => {
    expect(validateServerUrl('http://tools.example.com/mcp')).toMatch(/https/)
    expect(validateServerUrl('https://user:pw@x.example/')).toMatch(/user name/)
    expect(validateServerUrl('https://user@x.example/')).toMatch(/user name/)
    expect(validateServerUrl('https://x.example/#top')).toMatch(/fragment/)
    expect(validateServerUrl('not a url')).toMatch(/not a URL/)
    expect(validateServerUrl('')).toMatch(/Enter/)
  })

  it('refuses private, loopback, link-local and metadata addresses', () => {
    for (const url of [
      'https://10.0.0.5/mcp',
      'https://127.0.0.1/',
      'https://169.254.169.254/latest',
      'https://192.168.1.10:8443/',
      'https://172.16.0.1/',
      'https://100.64.0.1/',
      'https://0.0.0.0/',
      'https://[::1]/',
      'https://[fd00:ec2::254]/',
      'https://[fe80::1]/',
      'https://[::ffff:10.0.0.1]/',
      'https://0x7f.1/',
      'https://2130706433/',
    ]) {
      expect(validateServerUrl(url), url).toMatch(/private, loopback or link-local/)
    }
  })

  it('refuses hosts that only make sense inside a network', () => {
    for (const url of [
      'https://localhost/mcp',
      'https://autharie-mcp/mcp',
      'https://crm.local/',
      'https://crm.internal/',
      'https://api.default.svc.cluster.local/',
      'https://ferriskey-mcp.autharie-system.svc/mcp',
      'https://nas.home.arpa/',
    ]) {
      expect(validateServerUrl(url), url).toMatch(/internal/)
    }
  })

  it('refuses a URL that is too long', () => {
    expect(validateServerUrl(`https://x.example/${'a'.repeat(2100)}`)).toMatch(/at most/)
  })
})

describe('the slug of a customer server', () => {
  it('follows the control plane rule', () => {
    for (const slug of ['crm', 'billing-v2', 'ab']) expect(validateSlug(slug), slug).toBeNull()
    for (const slug of ['', 'a', '1crm', 'CRM', 'crm_x', 'crm.x', 'a'.repeat(32)]) {
      expect(validateSlug(slug), slug).not.toBeNull()
    }
  })

  it('reserves the names of the built-in servers and of the gateway client', () => {
    for (const slug of ['ferriskey', 'autharie', 'gateway']) {
      expect(validateSlug(slug)).toMatch(/reserved/)
    }
  })

  it('is derived from the name', () => {
    expect(slugFromName('Billing CRM')).toBe('billing-crm')
    expect(slugFromName('  Café & Co. ')).toBe('caf-co')
    expect(slugFromName('2024 ERP')).toBe('s-2024-erp')
    expect(slugFromName('!!!')).toBe('')
    expect(slugFromName('x'.repeat(60))).toHaveLength(31)
    expect(validateSlug(slugFromName('Billing CRM'))).toBeNull()
  })
})

describe('the name of a customer server', () => {
  it('is 1 to 100 characters once trimmed', () => {
    expect(validateName(' CRM ')).toBeNull()
    expect(validateName('   ')).not.toBeNull()
    expect(validateName('n'.repeat(101))).not.toBeNull()
  })
})
