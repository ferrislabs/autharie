import type { SVGProps } from 'react'
import { cn } from '@/lib/utils'
import { KIND_LABELS, type DeploymentKind } from '../../../types/deployment'

function FerrisKeyLogo({ className }: { className?: string }) {
  return (
    <img
      src='/logos/ferriskey.png'
      alt=''
      aria-hidden='true'
      className={cn('object-contain', className)}
    />
  )
}

function KeycloakLogo({ className, ...props }: SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox='0 0 24 24'
      aria-hidden='true'
      fill='currentColor'
      className={cn('text-[#4d4d4d] dark:text-[#cfcfcf]', className)}
      {...props}
    >
      <path d='m18.742 1.182-12.493.002C4.155 4.784 2.079 8.393 0 12.002c2.071 3.612 4.162 7.214 6.252 10.816l12.49-.004 3.089-5.404h2.158v-.002H24L23.996 6.59h-2.168zM8.327 4.792h2.081l1.04 1.8-3.12 5.413 3.117 5.403-1.035 1.81H8.327a2047.566 2047.566 0 0 0-4.168-7.204C5.547 9.606 6.937 7.2 8.327 4.792Zm6.241 0 2.086.003c1.393 2.405 2.78 4.813 4.166 7.222l-4.167 7.2h-2.08c-.382-.562-1.038-1.808-1.038-1.808l3.123-5.405-3.124-5.413z' />
    </svg>
  )
}

export function DeploymentKindLogo({
  kind,
  className,
}: {
  kind: DeploymentKind
  className?: string
}) {
  return kind === 'ferriskey' ? (
    <FerrisKeyLogo className={className} />
  ) : (
    <KeycloakLogo className={className} />
  )
}

export function DeploymentKindLabel({ kind }: { kind: DeploymentKind }) {
  return (
    <span className='inline-flex items-center gap-2'>
      <DeploymentKindLogo kind={kind} className='size-6 shrink-0' />
      {KIND_LABELS[kind]}
    </span>
  )
}
