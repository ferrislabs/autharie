import { useState } from 'react'
import type { Schemas } from '@/api/api.client'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import { formFrom, isFilled, requestFrom, type AccountForm } from './account-form-state'

export const NEEDS_MANAGE_TO_EDIT = 'Editing the billing details needs the Manage billing permission.'

interface Props {
  account: Schemas.BillingAccountResponse | null
  canManage: boolean
  isSaving: boolean
  refusal?: string
  onSave: (request: Schemas.BillingAccountRequest) => void
}

type Field = {
  key: keyof AccountForm
  label: string
  type?: string
  autoComplete?: string
  maxLength?: number
  wide?: boolean
  optional?: boolean
}

const FIELDS: ReadonlyArray<Field> = [
  { key: 'name', label: 'Name', autoComplete: 'organization', wide: true },
  { key: 'email', label: 'E-mail', type: 'email', autoComplete: 'email', wide: true },
  { key: 'line1', label: 'Address line 1', autoComplete: 'address-line1', wide: true },
  { key: 'line2', label: 'Address line 2', autoComplete: 'address-line2', wide: true, optional: true },
  { key: 'postal_code', label: 'Postal code', autoComplete: 'postal-code' },
  { key: 'city', label: 'City', autoComplete: 'address-level2' },
  { key: 'country', label: 'Country (ISO code, e.g. FR)', autoComplete: 'country', maxLength: 2 },
  { key: 'vat_number', label: 'VAT number', optional: true },
  { key: 'siren', label: 'SIREN', optional: true },
]

export function BillingAccountForm({ account, canManage, isSaving, refusal, onSave }: Props) {
  const [form, setForm] = useState<AccountForm>(() => formFrom(account))

  return (
    <form
      className='space-y-4 rounded-lg border bg-card p-5'
      onSubmit={(event) => {
        event.preventDefault()
        if (!canManage || isSaving || !isFilled(form)) return
        onSave(requestFrom(form))
      }}
    >
      <div className='grid gap-4 sm:grid-cols-2'>
        {FIELDS.map((field) => {
          const id = `billing-${field.key}`

          return (
            <div key={field.key} className={field.wide ? 'space-y-2 sm:col-span-2' : 'space-y-2'}>
              <Label htmlFor={id}>
                {field.label}
                {field.optional && <span className='text-muted-foreground'> (optional)</span>}
              </Label>
              <Input
                id={id}
                type={field.type ?? 'text'}
                autoComplete={field.autoComplete}
                maxLength={field.maxLength}
                value={form[field.key]}
                disabled={!canManage || isSaving}
                onChange={(event) => setForm({ ...form, [field.key]: event.target.value })}
              />
            </div>
          )
        })}
      </div>

      {refusal && (
        <p role='alert' className='text-sm text-destructive'>
          {refusal}
        </p>
      )}

      <div className='flex items-center justify-between gap-4'>
        {!canManage && <p className='text-xs text-muted-foreground'>{NEEDS_MANAGE_TO_EDIT}</p>}
        <Button
          type='submit'
          size='sm'
          className='ml-auto'
          disabled={!canManage || isSaving || !isFilled(form)}
          title={canManage ? undefined : NEEDS_MANAGE_TO_EDIT}
        >
          {isSaving && <Spinner className='size-3.5' />}
          Save
        </Button>
      </div>
    </form>
  )
}
