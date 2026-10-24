import { Globe, LayoutTemplate, Mail, MailPlus, MessageSquareText, Smartphone } from 'lucide-react'
import type { SideNavEntry } from './side-nav'

/**
 * The entries of the Branding side navigation, in Auth0's order so a customer
 * coming from there finds things where they expect them.
 *
 * `soon` entries are announced and not built: they are not links.
 */
export function brandingEntries(realmPath: (path: string) => string): SideNavEntry[] {
  return [
    {
      label: 'Universal login',
      to: realmPath('/branding/universal-login'),
      icon: LayoutTemplate,
    },
    {
      label: 'Custom domains',
      to: realmPath('/branding/custom-domains'),
      icon: Globe,
      soon: true,
    },
    {
      label: 'Email templates',
      to: realmPath('/branding/email-templates'),
      icon: Mail,
      soon: true,
    },
    {
      label: 'Email provider',
      to: realmPath('/branding/email-provider'),
      icon: MailPlus,
    },
    {
      label: 'Phone templates',
      to: realmPath('/branding/phone-templates'),
      icon: MessageSquareText,
      soon: true,
    },
    {
      label: 'Phone provider',
      to: realmPath('/branding/phone-provider'),
      icon: Smartphone,
      soon: true,
    },
  ]
}
