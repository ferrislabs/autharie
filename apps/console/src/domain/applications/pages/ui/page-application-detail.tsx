import { Link } from '@tanstack/react-router'
import { ArrowLeft } from 'lucide-react'
import { InfoRow, Page, PageTitle, Section } from '@/components/layout/page'
import { StatusBadge } from '@/components/ui/status-badge'
import { isAgentApplication } from '../../agent-application'
import { formatSeconds, formatWhen, humanise } from '../../application-labels'
import type { ApplicationDetail } from '../../types'
import { AgentBadge } from './agent-badge'
import { RequestNotice } from './request-notice'

interface Props {
  realm: string
  application: ApplicationDetail | null
  isLoading: boolean
  isSlow: boolean
  error?: string
  backPath: string
  agentsPath: string
  onRetry: () => void
}

const yesNo = (value: boolean) => (value ? 'Yes' : 'No')

function lifetimes(application: ApplicationDetail): Array<[string, number]> {
  const all: Array<[string, number | undefined]> = [
    ['Access token', application.access_token_lifetime],
    ['Refresh token', application.refresh_token_lifetime],
    ['ID token', application.id_token_lifetime],
  ]
  return all.filter((entry): entry is [string, number] => entry[1] !== undefined)
}

export const PageApplicationDetail = ({
  realm,
  application,
  isLoading,
  isSlow,
  error,
  backPath,
  agentsPath,
  onRetry,
}: Props) => {
  const back = (
    <Link
      to={backPath}
      className='mb-4 inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground'
    >
      <ArrowLeft className='h-4 w-4' />
      Applications
    </Link>
  )

  if (application === null) {
    return (
      <Page>
        {back}
        <PageTitle title='Application' />
        <div className='mt-6'>
          <RequestNotice
            what='the application'
            isLoading={isLoading}
            isSlow={isSlow}
            error={error}
            onRetry={onRetry}
          />
        </div>
      </Page>
    )
  }

  const times = lifetimes(application)

  return (
    <Page>
      {back}
      <PageTitle
        title={application.name}
        badges={
          <>
            <StatusBadge tone={application.enabled ? 'success' : 'neutral'}>
              {application.enabled ? 'Enabled' : 'Disabled'}
            </StatusBadge>
            <StatusBadge tone='neutral' dot={false}>
              {humanise(application.client_type)}
            </StatusBadge>
            <StatusBadge tone='neutral' dot={false}>
              {realm}
            </StatusBadge>
            {isAgentApplication(application) && <AgentBadge agentsPath={agentsPath} />}
          </>
        }
      />

      <div className='mt-6 grid gap-8 md:grid-cols-2'>
        <Section title='General' className='rounded-lg border p-5'>
          <div className='space-y-3'>
            <InfoRow
              label='Client ID'
              value={<span className='font-mono text-xs'>{application.client_id}</span>}
            />
            <InfoRow
              label='ID'
              value={<span className='font-mono text-xs'>{application.id}</span>}
            />
            <InfoRow label='Protocol' value={humanise(application.protocol)} />
            <InfoRow label='Type' value={humanise(application.client_type)} />
            <InfoRow label='Registered from' value={humanise(application.registration_source)} />
            <InfoRow label='Created' value={formatWhen(application.created_at)} />
            <InfoRow label='Updated' value={formatWhen(application.updated_at)} />
          </div>
        </Section>

        <Section title='Sign-in' className='rounded-lg border p-5'>
          <div className='space-y-3'>
            <InfoRow label='Public client' value={yesNo(application.public_client)} />
            <InfoRow label='Service account' value={yesNo(application.service_account_enabled)} />
            <InfoRow
              label='Direct access grants'
              value={yesNo(application.direct_access_grants_enabled)}
            />
            <InfoRow label='PKCE required' value={yesNo(application.require_pkce)} />
            <InfoRow label='Token exchange' value={yesNo(application.token_exchange_enabled)} />
            <InfoRow label='Consent required' value={yesNo(application.consent_required)} />
          </div>
        </Section>

        <Section title='Redirect URIs' className='rounded-lg border p-5'>
          {application.redirect_uris.length === 0 ? (
            <p className='text-sm text-muted-foreground'>No redirect URI.</p>
          ) : (
            <ul className='space-y-1.5'>
              {application.redirect_uris.map((uri) => (
                <li key={uri} className='break-all font-mono text-xs'>
                  {uri}
                </li>
              ))}
            </ul>
          )}
        </Section>

        {times.length > 0 && (
          <Section title='Token lifetimes' className='rounded-lg border p-5'>
            <div className='space-y-3'>
              {times.map(([label, seconds]) => (
                <InfoRow key={label} label={label} value={formatSeconds(seconds)} />
              ))}
            </div>
          </Section>
        )}
      </div>
    </Page>
  )
}
