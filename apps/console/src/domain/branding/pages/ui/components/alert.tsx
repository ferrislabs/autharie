export function Alert({ children }: { children: React.ReactNode }) {
  return (
    <p
      role='alert'
      className='rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive'
    >
      {children}
    </p>
  )
}

export function Notice({ title, children }: { title: string; children?: React.ReactNode }) {
  return (
    <div className='rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'>
      <p className='font-medium'>{title}</p>
      {children && <p className='mt-0.5'>{children}</p>}
    </div>
  )
}
