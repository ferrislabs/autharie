import {
  Bot,
  Copy,
  Eye,
  FileText,
  Globe,
  KeyRound,
  Pencil,
  Plus,
  Power,
  Server,
  Trash2,
  Unlink,
  User,
  Users,
  Wrench,
} from 'lucide-react'
import type { Subject, Target } from '../../../canvas-model'
import { POLICY_TEMPLATES, type PolicyTemplate } from '../../../policy-templates'
import { SERVERS } from '../../../tool-catalogue'
import type { MenuItem } from './canvas-context-menu'

export type Restrict = 'all' | 'rule' | 'what'

export interface AddActions {
  newRule: () => void
  addWho: (subject: Subject, edit: boolean) => void
  addWhat: (target: Target, edit: boolean) => void
  fromTemplate: (template: PolicyTemplate) => void
}

export function templateItems(actions: AddActions): MenuItem[] {
  return POLICY_TEMPLATES.map((template) => ({
    id: `template:${template.id}`,
    label: template.title,
    icon: FileText,
    onSelect: () => actions.fromTemplate(template),
  }))
}

function whoItems(actions: AddActions): MenuItem[] {
  return [
    { id: 'who:any', label: 'Any user', icon: Globe, onSelect: () => actions.addWho({ type: 'any' }, false) },
    { id: 'who:group', label: 'Group…', icon: Users, onSelect: () => actions.addWho({ type: 'group', value: '' }, true) },
    { id: 'who:role', label: 'Role…', icon: KeyRound, onSelect: () => actions.addWho({ type: 'role', value: '' }, true) },
    { id: 'who:user', label: 'User…', icon: User, onSelect: () => actions.addWho({ type: 'user', value: '' }, true) },
    {
      id: 'who:agent',
      label: 'Autonomous agent…',
      icon: Bot,
      onSelect: () => actions.addWho({ type: 'agent', value: '' }, true),
    },
  ]
}

export function whatItems(actions: AddActions): MenuItem[] {
  return [
    {
      id: 'what:read-only',
      label: 'Read-only tools',
      icon: Eye,
      onSelect: () => actions.addWhat({ type: 'read-only', server: null }, false),
    },
    ...SERVERS.map((server) => ({
      id: `what:read-only:${server.id}`,
      label: `Read-only on ${server.label}`,
      icon: Eye,
      onSelect: () => actions.addWhat({ type: 'read-only', server: server.id }, false),
    })),
    {
      id: 'what:server',
      label: 'Whole server',
      icon: Server,
      items: SERVERS.map((server) => ({
        id: `what:server:${server.id}`,
        label: server.label,
        onSelect: () => actions.addWhat({ type: 'server', server: server.id }, false),
      })),
    },
    {
      id: 'what:tools',
      label: 'Pick tools…',
      icon: Wrench,
      onSelect: () => actions.addWhat({ type: 'tools', tools: [] }, true),
    },
  ]
}

export function paneMenu(actions: AddActions, restrict: Restrict): MenuItem[] {
  if (restrict === 'rule') return [{ id: 'rule:new', label: 'New rule', icon: Plus, onSelect: actions.newRule }]
  if (restrict === 'what') return whatItems(actions)
  return [
    { id: 'rule:new', label: 'New rule', icon: Plus, onSelect: actions.newRule },
    { id: 'who', label: 'Add who', icon: Users, items: whoItems(actions), separatorBefore: true },
    { id: 'what', label: 'Add what', icon: Wrench, items: whatItems(actions) },
    { id: 'template', label: 'From template', icon: FileText, items: templateItems(actions), separatorBefore: true },
  ]
}

export interface BoxActions {
  edit: () => void
  duplicate?: () => void
  toggle?: { enabled: boolean; run: () => void }
  remove: () => void
}

export function boxMenu(actions: BoxActions): MenuItem[] {
  const items: MenuItem[] = [{ id: 'edit', label: 'Edit', icon: Pencil, onSelect: actions.edit }]
  if (actions.duplicate) {
    items.push({ id: 'duplicate', label: 'Duplicate', icon: Copy, onSelect: actions.duplicate })
  }
  if (actions.toggle) {
    items.push({
      id: 'toggle',
      label: actions.toggle.enabled ? 'Disable' : 'Enable',
      icon: Power,
      onSelect: actions.toggle.run,
    })
  }
  items.push({
    id: 'delete',
    label: 'Delete',
    icon: Trash2,
    destructive: true,
    separatorBefore: true,
    onSelect: actions.remove,
  })
  return items
}

export function linkMenu(remove: () => void): MenuItem[] {
  return [{ id: 'unlink', label: 'Delete link', icon: Unlink, destructive: true, onSelect: remove }]
}
