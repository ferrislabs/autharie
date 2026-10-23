import { Globe, LayoutTemplate, Mail, MailPlus, MessageSquareText, Smartphone } from 'lucide-react'
import type { SideNavEntry } from './side-nav'

/**
 * The entries of the Branding side navigation, in Auth0's order so a customer
 * coming from there finds things where they expect them.
 *
 * `soon` entries are announced and not built: they are not links.
 */
export function brandingEntries(deploymentPath: (path: string) => string): SideNavEntry[] {
  return [
    {
      label: 'Universal login',
      to: deploymentPath('/branding/universal-login'),
      icon: LayoutTemplate,
    },
    {
      label: 'Custom domains',
      to: deploymentPath('/branding/custom-domains'),
      icon: Globe,
      soon: true,
    },
    {
      label: 'Email templates',
      to: deploymentPath('/branding/email-templates'),
      icon: Mail,
      soon: true,
    },
    {
      label: 'Email provider',
      to: deploymentPath('/branding/email-provider'),
      icon: MailPlus,
    },
    {
      label: 'Phone templates',
      to: deploymentPath('/branding/phone-templates'),
      icon: MessageSquareText,
      soon: true,
    },
    {
      label: 'Phone provider',
      to: deploymentPath('/branding/phone-provider'),
      icon: Smartphone,
      soon: true,
    },
  ]
}
