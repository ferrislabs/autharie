import { Link } from '@tanstack/react-router'
import { AppWindow, ChevronLeft, ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { EmptyState, Page, PageTitle } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { StatusBadge } from '@/components/ui/status-badge'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { isAgentApplication } from '../../agent-application'
import { humanise } from '../../application-labels'
import type { ApplicationSummary } from '../../types'
import { AgentBadge } from './agent-badge'
import { RequestNotice } from './request-notice'

interface Props {
  realm: string
  applications: ApplicationSummary[] | null
  isLoading: boolean
  isSlow: boolean
  error?: string
  search: string
  page: number
  hasNext: boolean
  canGoPrevious: boolean
  detailPath: (id: string) => string
  agentsPath: string
  onSearch: (term: string) => void
  onNext: () => void
  onPrevious: () => void
  onRetry: () => void
}

export const PageApplications = ({
  realm,
  applications,
  isLoading,
  isSlow,
  error,
  search,
  page,
  hasNext,
  canGoPrevious,
  detailPath,
  agentsPath,
  onSearch,
  onNext,
  onPrevious,
  onRetry,
}: Props) => {
  const [draft, setDraft] = useState(search)
  const showPager = error === undefined && (page > 1 || hasNext)

  return (
    <Page>
      <PageTitle
        title='Applications'
        badges={
          <StatusBadge tone='neutral' dot={false}>
            {realm}
          </StatusBadge>
        }
      />

      <div className='mt-6 space-y-4'>
        <form
          role='search'
          className='max-w-sm'
          onSubmit={(event) => {
            event.preventDefault()
            onSearch(draft)
          }}
        >
          <Input
            type='search'
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            placeholder='Search by name or client id, then press Enter'
            aria-label='Search applications'
          />
        </form>

        <RequestNotice
          what='applications'
          isLoading={isLoading}
          isSlow={isSlow}
          error={error}
          onRetry={onRetry}
        />

        {applications !== null && applications.length === 0 && (
          <EmptyState
            icon={<AppWindow className='h-5 w-5' />}
            title={search === '' ? 'No applications in this realm' : `No application matches “${search}”`}
            description={
              search === ''
                ? 'Applications registered in this realm will show up here.'
                : 'Try another search, or clear it to see every application.'
            }
          />
        )}

        {applications !== null && applications.length > 0 && (
          <div className='overflow-x-auto rounded-lg border'>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Client ID</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Source</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {applications.map((application) => (
                  <TableRow key={application.id}>
                    <TableCell className='font-medium'>
                      <Link to={detailPath(application.id)} className='hover:underline'>
                        {application.name}
                      </Link>
                    </TableCell>
                    <TableCell>
                      <span className='flex items-center gap-2'>
                        <span className='font-mono text-xs text-muted-foreground'>
                          {application.client_id}
                        </span>
                        {isAgentApplication(application) && <AgentBadge agentsPath={agentsPath} />}
                      </span>
                    </TableCell>
                    <TableCell className='text-muted-foreground'>
                      {humanise(application.client_type)}
                    </TableCell>
                    <TableCell>
                      <StatusBadge tone={application.enabled ? 'success' : 'neutral'}>
                        {application.enabled ? 'Enabled' : 'Disabled'}
                      </StatusBadge>
                    </TableCell>
                    <TableCell className='text-muted-foreground'>
                      {humanise(application.registration_source)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}

        {showPager && (
          <nav aria-label='Pages' className='flex items-center justify-between gap-4'>
            <span className='text-sm text-muted-foreground'>Page {page}</span>
            <div className='flex items-center gap-2'>
              <Button
                variant='outline'
                size='sm'
                onClick={onPrevious}
                disabled={!canGoPrevious || isLoading}
              >
                <ChevronLeft className='h-4 w-4' />
                Previous
              </Button>
              <Button variant='outline' size='sm' onClick={onNext} disabled={!hasNext || isLoading}>
                Next page
                <ChevronRight className='h-4 w-4' />
              </Button>
            </div>
          </nav>
        )}
      </div>
    </Page>
  )
}
