import { toolId, type Catalogue } from './tool-catalogue'
import type { RuleModel } from './types'

export interface PolicyTemplateDraft {
  name: string
  description: string
  rule: RuleModel
}

export interface PolicyTemplate {
  id: string
  title: string
  description: string
  build: (catalogue: Catalogue) => PolicyTemplateDraft
}

export const POLICY_TEMPLATES: PolicyTemplate[] = [
  {
    id: 'read-only',
    title: 'Read-only for everyone',
    description: 'Any user, through any agent, can look but not change anything.',
    build: () => ({
      name: 'Read-only for everyone',
      description: 'Agents may call read-only tools on every server, for any user.',
      rule: {
        version: 1,
        effect: 'permit',
        who: { kind: 'any-user' },
        agents: { kind: 'any' },
        what: { kind: 'read-only', server: null },
        when: null,
      },
    }),
  },
  {
    id: 'block-destructive',
    title: 'Block destructive tools',
    description: 'A safety net: disabling users, removing roles or deleting is always refused.',
    build: (catalogue) => ({
      name: 'Block destructive tools',
      description: 'Refuses every tool marked destructive, whatever other policies permit.',
      rule: {
        version: 1,
        effect: 'forbid',
        who: { kind: 'any-user' },
        agents: { kind: 'any' },
        what: {
          kind: 'tools',
          tools: catalogue.tools.filter((tool) => tool.destructive).map(toolId),
        },
        when: null,
      },
    }),
  },
  {
    id: 'admins-ferriskey',
    title: 'Admins can manage FerrisKey',
    description: 'Users with the admin role get every FerrisKey tool.',
    build: () => ({
      name: 'Admins can manage FerrisKey',
      description: 'Users with the admin role may call every FerrisKey tool through any agent.',
      rule: {
        version: 1,
        effect: 'permit',
        who: { kind: 'roles', roles: ['admin'] },
        agents: { kind: 'any' },
        what: { kind: 'server', server: 'ferriskey' },
        when: null,
      },
    }),
  },
  {
    id: 'read-only-business-hours',
    title: 'Read-only during business hours',
    description: 'Read-only access Monday to Friday, 09:00 to 18:00 UTC.',
    build: () => ({
      name: 'Read-only during business hours',
      description: 'Read-only tools for any user, Monday to Friday between 09:00 and 18:00 UTC.',
      rule: {
        version: 1,
        effect: 'permit',
        who: { kind: 'any-user' },
        agents: { kind: 'any' },
        what: { kind: 'read-only', server: null },
        when: { weekdays: [1, 2, 3, 4, 5], hours: { from: 9, to: 18 } },
      },
    }),
  },
]
