import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { ProvisioningNoticeBanner } from './pages/ui/provisioning-notice'
import { FAILED_WITHOUT_REASON, provisioningNotice, stillProvisioning } from './provisioning'

const customer: Pick<Schemas.Deployment, 'distribution'> = {
  distribution: {
    customer_cloud: {
      credential_id: 'c',
      profile: {
        control_plane: { id: 'm', kind: 'mutualized', monthly_price: 0 },
        max_nodes: 1,
        min_nodes: 1,
        mode: 'dev',
        node_type: 'DEV1-M',
        replication: 1,
      },
    },
  },
}

describe('what a customer cloud deployment shows', () => {
  it('says the cluster is being created while the plane provisions', () => {
    const notice = provisioningNotice(customer, { status: 'provisioning' })

    expect(notice).toEqual({ kind: 'creating', message: 'Creating your cluster' })
    expect(renderToStaticMarkup(<ProvisioningNoticeBanner notice={notice!} />)).toContain(
      'Creating your cluster',
    )
  })

  it('shows the failure reason when the plane failed', () => {
    const notice = provisioningNotice(customer, {
      status: 'failed',
      failure_reason: 'quota exceeded for DEV1-M in fr-par',
    })

    expect(notice).toEqual({ kind: 'failed', message: 'quota exceeded for DEV1-M in fr-par' })
    expect(renderToStaticMarkup(<ProvisioningNoticeBanner notice={notice!} />)).toContain(
      'quota exceeded for DEV1-M in fr-par',
    )
  })

  it('still says something when a failed plane gave no reason', () => {
    expect(provisioningNotice(customer, { status: 'failed', failure_reason: null })).toEqual({
      kind: 'failed',
      message: FAILED_WITHOUT_REASON,
    })
  })

  it('shows nothing for an active plane, a shared deployment or an unread plane', () => {
    expect(provisioningNotice(customer, { status: 'ready' })).toBeNull()
    expect(provisioningNotice({ distribution: 'shared' }, { status: 'provisioning' })).toBeNull()
    expect(provisioningNotice(customer, undefined)).toBeNull()
    expect(provisioningNotice(customer, null)).toBeNull()
  })

  it('keeps polling only while provisioning', () => {
    expect(stillProvisioning({ status: 'provisioning' })).toBe(true)
    expect(stillProvisioning({ status: 'failed' })).toBe(false)
    expect(stillProvisioning(undefined)).toBe(false)
  })
})
