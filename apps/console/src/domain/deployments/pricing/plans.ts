import type { Engine, Hosting, PlanId } from './model'

export interface Feature {
  text: string
  ferriskeyOnly?: boolean
}

export interface PlanCopy {
  name: string
  tagline: string
  features: Feature[]
}

const only = (text: string): Feature => ({ text, ferriskeyOnly: true })

export const PLAN_ORDER: PlanId[] = ['starter', 'business', 'scale']

export const PLANS: Record<Hosting, Record<PlanId, PlanCopy>> = {
  managed: {
    starter: {
      name: 'Starter',
      tagline: 'Ideal for an MVP or internal applications.',
      features: [
        { text: '99% availability' },
        { text: 'Email support, next business day (best effort)' },
        { text: 'Audit history: 7 days' },
        { text: 'One-click updates, with rollback' },
      ],
    },
    business: {
      name: 'Business',
      tagline: 'For products in production.',
      features: [
        { text: '99.5% availability' },
        { text: 'Guaranteed priority support (under 4 h)' },
        {
          text: 'Ephemeral staging sandbox: a test environment in one click, on a copy of your data, to validate breaking changes and version upgrades before production',
        },
        { text: 'Audit and analytics: 30 days (managed Quickwit / Datafusion)' },
        only('Distributed authorisations (ReBAC) included'),
        only('MCP gateway (AI and agents) included'),
      ],
    },
    scale: {
      name: 'Scale',
      tagline: 'To go further, with the highest level of support.',
      features: [
        { text: '99.9% availability, multi-AZ, high availability' },
        {
          text: 'Highest support: dedicated Slack or Teams channel, critical 24/7 on-call (under 1 h)',
        },
        { text: 'Unlimited staging sandboxes, with automated cloning' },
        {
          text: 'Advanced analytics: search across all audit and sign-in logs, usage dashboards',
        },
        { text: 'Audit retention from 90 to 365 days, and compliance exports' },
        only('Distributed authorisations (ReBAC) included'),
        only('MCP gateway (AI and agents) included'),
      ],
    },
  },
  byoc: {
    starter: {
      name: 'BYOC Starter',
      tagline: 'The control plane with us, the data with you.',
      features: [
        { text: '99% control plane availability' },
        { text: 'Email support' },
        { text: 'Raw log export' },
      ],
    },
    business: {
      name: 'BYOC Business',
      tagline: 'For production in your cloud.',
      features: [
        { text: '99.5% control plane availability' },
        { text: 'Support in under 4 h' },
        { text: 'Ephemeral staging sandbox, orchestrated in your cloud' },
        { text: 'Managed Quickwit deployment in your account' },
        only('ReBAC and MCP included'),
      ],
    },
    scale: {
      name: 'BYOC Scale',
      tagline: 'Several clusters, the highest level of support.',
      features: [
        { text: '99.9% control plane availability' },
        { text: 'Multi-cluster' },
        { text: '24/7 support, with a dedicated channel' },
        { text: 'Unlimited staging sandboxes, orchestrated in your clouds' },
        { text: 'Advanced analytics: search across all logs, usage dashboards' },
        { text: 'Automated canary deployments' },
        only('ReBAC and MCP included'),
      ],
    },
  },
}

export const ENGINES: Record<Engine, { name: string; badge: string; description: string }> = {
  ferriskey: {
    name: 'FerrisKey',
    badge: 'Recommended',
    description: 'Cloud-native & ultra-light',
  },
  keycloak: {
    name: 'Keycloak',
    badge: '+30% infrastructure cost',
    description: 'The historical JVM-based engine, which needs more memory and compute.',
  },
}

export const KEYCLOAK_NOTICE = {
  title: 'With Keycloak, some features are not available',
  warning: 'Distributed ReBAC and the MCP Hub are not available on this engine.',
  intro: 'These features of the plans below are not included with this engine:',
  switch: 'Switch to FerrisKey',
}

export const BYOC_NOTE = {
  title: 'Unlimited users',
  text: 'You pay your infrastructure directly to your cloud provider. Autharie charges a fixed fee for the control plane and the orchestration, whatever the number of accounts.',
}

export const VOLUME_NOTES = {
  free: 'Discovery: free up to 1,000 accounts with FerrisKey.',
  keycloakNoFree:
    'The free offer is reserved for the FerrisKey engine. With Keycloak, pricing starts at €49 a month.',
  businessFloor: 'Business starts at the price of the 10,000 accounts tier.',
}

export function ferriskeyOnlyFeatures(hosting: Hosting): string[] {
  return [
    ...new Set(
      PLAN_ORDER.flatMap((id) =>
        PLANS[hosting][id].features.filter((f) => f.ferriskeyOnly).map((f) => f.text),
      ),
    ),
  ]
}

export function volumeNotes(hosting: Hosting, engine: Engine, volume: number): string[] {
  if (hosting !== 'managed') return []
  const notes: string[] = []
  if (volume === 1000 && engine === 'ferriskey') notes.push(VOLUME_NOTES.free)
  if (volume === 1000 && engine === 'keycloak') notes.push(VOLUME_NOTES.keycloakNoFree)
  if (volume < 10000) notes.push(VOLUME_NOTES.businessFloor)
  return notes
}
