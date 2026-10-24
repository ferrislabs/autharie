import { Link } from '@tanstack/react-router'
import { Boxes, ShieldCheck } from 'lucide-react'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { cn } from '@/lib/utils'

export type Space = 'infrastructure' | 'identity'

function Side({
  to,
  active,
  position,
  children,
}: {
  to: string
  active: boolean
  position: 'first' | 'last'
  children: React.ReactNode
}) {
  return (
    <Link
      to={to}
      aria-current={active ? 'page' : undefined}
      className={cn(
        'flex items-center gap-1.5 border px-2.5 py-1.5 text-sm transition-colors',
        position === 'first' ? 'rounded-l-md' : '-ml-px rounded-r-md',
        active
          ? 'relative z-10 border-primary/40 bg-accent font-medium text-primary'
          : 'text-muted-foreground hover:bg-accent hover:text-foreground',
      )}
    >
      {children}
    </Link>
  )
}

/**
 * The two spaces of a deployment: running it, and what its users live with.
 *
 * Named Infrastructure and not Platform because `/platform` is the section for
 * whoever runs the installation, a different audience entirely.
 */
export function SpaceSwitch({ active }: { active: Space }) {
  const deploymentPath = useDeploymentPath()

  return (
    <div className='flex'>
      <Side to={deploymentPath()} active={active === 'infrastructure'} position='first'>
        <Boxes className='h-3.5 w-3.5' />
        Infrastructure
      </Side>
      <Side to={deploymentPath('/identity')} active={active === 'identity'} position='last'>
        <ShieldCheck className='h-3.5 w-3.5' />
        Identity
      </Side>
    </div>
  )
}
