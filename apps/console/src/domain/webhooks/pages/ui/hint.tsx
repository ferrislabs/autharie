import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

export function Hint({
  tip,
  children,
  className,
}: {
  tip: React.ReactNode
  children: React.ReactNode
  className?: string
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className={className}>{children}</span>
      </TooltipTrigger>
      <TooltipContent className='max-w-sm break-words'>{tip}</TooltipContent>
    </Tooltip>
  )
}
