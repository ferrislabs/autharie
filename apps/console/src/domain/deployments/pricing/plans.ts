import type { Engine, Hosting, PlanId } from './model'

export type Cell = boolean | string

export interface FeatureRow {
  label: string
  hint?: string
  cells: Record<PlanId, Cell>
  ferriskeyOnly?: boolean
}

export interface PlanCopy {
  name: string
  tagline: string
}

export const PLAN_ORDER: PlanId[] = ['starter', 'business', 'scale']

export const PLANS: Record<Hosting, Record<PlanId, PlanCopy>> = {
  managed: {
    starter: { name: 'Starter', tagline: 'Ideal for an MVP or internal applications.' },
    business: { name: 'Business', tagline: 'For products in production.' },
    scale: {
      name: 'Scale',
      tagline: 'To go further, with the highest level of support.',
    },
  },
  byoc: {
    starter: { name: 'BYOC Starter', tagline: 'The control plane with us, the data with you.' },
    business: { name: 'BYOC Business', tagline: 'For production in your cloud.' },
    scale: {
      name: 'BYOC Scale',
      tagline: 'Several clusters, the highest level of support.',
    },
  },
}

export const FEATURES: Record<Hosting, FeatureRow[]> = {
  managed: [
    {
      label: 'Availability',
      cells: { starter: '99%', business: '99.5%', scale: '99.9%, multi-AZ, high availability' },
    },
    {
      label: 'Support',
      cells: {
        starter: 'Email, next business day (best effort)',
        business: 'Guaranteed priority support (under 4 h)',
        scale: 'Dedicated Slack or Teams channel, critical 24/7 on-call (under 1 h)',
      },
    },
    {
      label: 'Audit history',
      cells: { starter: '7 days', business: '30 days', scale: '90 to 365 days, with compliance exports' },
    },
    {
      label: 'Analytics',
      cells: {
        starter: false,
        business: 'Managed Quickwit / Datafusion',
        scale: 'Advanced: search across all audit and sign-in logs, usage dashboards',
      },
    },
    {
      label: 'Staging sandbox',
      hint: 'A test environment on a copy of your data, to validate breaking changes and version upgrades before production.',
      cells: {
        starter: false,
        business: 'Ephemeral, in one click',
        scale: 'Unlimited, with automated cloning',
      },
    },
    {
      label: 'One-click updates, with rollback',
      cells: { starter: true, business: true, scale: true },
    },
    {
      label: 'Distributed authorisations (ReBAC)',
      cells: { starter: false, business: true, scale: true },
      ferriskeyOnly: true,
    },
    {
      label: 'MCP gateway (AI and agents)',
      cells: { starter: false, business: true, scale: true },
      ferriskeyOnly: true,
    },
  ],
  byoc: [
    {
      label: 'Control plane availability',
      cells: { starter: '99%', business: '99.5%', scale: '99.9%' },
    },
    {
      label: 'Support',
      cells: {
        starter: 'Email',
        business: 'Under 4 h',
        scale: '24/7, with a dedicated channel',
      },
    },
    {
      label: 'Logs',
      cells: {
        starter: 'Raw export',
        business: 'Managed Quickwit deployment in your account',
        scale: 'Advanced analytics: search across all logs, usage dashboards',
      },
    },
    {
      label: 'Staging sandbox',
      cells: {
        starter: false,
        business: 'Ephemeral, orchestrated in your cloud',
        scale: 'Unlimited, orchestrated in your clouds',
      },
    },
    { label: 'Multi-cluster', cells: { starter: false, business: false, scale: true } },
    {
      label: 'Automated canary deployments',
      cells: { starter: false, business: false, scale: true },
    },
    {
      label: 'ReBAC and MCP',
      cells: { starter: false, business: true, scale: true },
      ferriskeyOnly: true,
    },
  ],
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
  return FEATURES[hosting].filter((row) => row.ferriskeyOnly).map((row) => row.label)
}

export function volumeNotes(hosting: Hosting, engine: Engine, volume: number): string[] {
  if (hosting !== 'managed') return []
  const notes: string[] = []
  if (volume === 1000 && engine === 'ferriskey') notes.push(VOLUME_NOTES.free)
  if (volume === 1000 && engine === 'keycloak') notes.push(VOLUME_NOTES.keycloakNoFree)
  if (volume < 10000) notes.push(VOLUME_NOTES.businessFloor)
  return notes
}
