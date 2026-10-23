import type { CSSProperties } from 'react'
import { CircleAlert, KeyRound, Wand2 } from 'lucide-react'
import { previewVars } from '../../../theme-preview'
import type { Tokens } from '../../../theme-tokens'

type Weight = CSSProperties['fontWeight']

const widget: CSSProperties = {
  background: 'var(--fk-color-widget-bg)',
  color: 'var(--fk-color-body-text)',
  borderRadius: 'var(--fk-radius-widget)',
  borderWidth: 'var(--fk-border-widget)',
  borderStyle: 'solid',
  borderColor: 'rgba(0,0,0,0.12)',
  boxShadow: 'var(--fk-shadow-widget)',
  fontSize: 'var(--fk-font-body-size)',
  fontWeight: 'var(--fk-font-body-weight)' as Weight,
  padding: 'var(--fk-spacing-widget-padding)',
}

const title: CSSProperties = {
  fontSize: 'var(--fk-font-title-size)',
  fontWeight: 'var(--fk-font-title-weight)' as Weight,
  lineHeight: 1.2,
  margin: 0,
}

const subtitle: CSSProperties = {
  fontSize: 'var(--fk-font-subtitle-size)',
  fontWeight: 'var(--fk-font-subtitle-weight)' as Weight,
  opacity: 0.7,
  margin: 0,
}

const label: CSSProperties = {
  fontSize: 'var(--fk-font-input-label-size)',
  fontWeight: 'var(--fk-font-input-label-weight)' as Weight,
}

const input: CSSProperties = {
  borderRadius: 'var(--fk-radius-input)',
  borderWidth: 'var(--fk-border-input)',
  borderStyle: 'solid',
  borderColor: 'rgba(0,0,0,0.15)',
  background: 'transparent',
  color: 'var(--fk-color-body-text)',
}

const buttonText: CSSProperties = {
  fontSize: 'var(--fk-font-button-size)',
  fontWeight: 'var(--fk-font-button-weight)' as Weight,
  borderRadius: 'var(--fk-radius-button)',
  borderStyle: 'solid',
}

const primary: CSSProperties = {
  ...buttonText,
  background: 'var(--fk-color-primary-button)',
  color: 'var(--fk-color-primary-button-label)',
  borderWidth: 'var(--fk-border-button)',
  borderColor: 'transparent',
}

const secondary: CSSProperties = {
  ...buttonText,
  background: 'var(--fk-color-secondary-button)',
  color: 'var(--fk-color-secondary-button-label)',
  borderWidth: 'var(--fk-border-button)',
  borderColor: 'rgba(0,0,0,0.12)',
}

const social: CSSProperties = {
  ...buttonText,
  background: 'var(--fk-color-social-button-bg)',
  color: 'var(--fk-color-social-button-label)',
  borderWidth: 'var(--fk-border-social-button)',
  borderColor: 'var(--fk-color-social-button-border)',
}

const magicLink: CSSProperties = {
  ...buttonText,
  background: 'var(--fk-color-magic-link-button-bg)',
  color: 'var(--fk-color-magic-link-button-label)',
  borderWidth: 'var(--fk-border-magic-link-button)',
  borderColor: 'var(--fk-color-magic-link-button-border)',
}

const passkey: CSSProperties = {
  ...buttonText,
  background: 'var(--fk-color-passkey-button-bg)',
  color: 'var(--fk-color-passkey-button-label)',
  borderWidth: 'var(--fk-border-passkey-button)',
  borderColor: 'var(--fk-color-passkey-button-border)',
}

const link: CSSProperties = {
  color: 'var(--fk-color-links)',
  fontSize: 'var(--fk-font-link-size)',
  fontWeight: 'var(--fk-font-link-weight)' as Weight,
  textDecoration: 'var(--fk-font-link-decoration)' as CSSProperties['textDecoration'],
}

const errorLine: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  padding: '10px 12px',
  borderRadius: 'var(--fk-radius-input)',
  border: '1px solid var(--fk-color-error)',
  background: 'color-mix(in srgb, var(--fk-color-error) 10%, transparent)',
  color: 'var(--fk-color-error)',
  fontSize: 'var(--fk-font-body-size)',
}

const divider: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  opacity: 0.6,
}

const rule: CSSProperties = { flex: 1, height: 1, background: 'currentColor', opacity: 0.3 }

const column = (gap: string): CSSProperties => ({ display: 'flex', flexDirection: 'column', gap })

export function ThemePreview({ tokens }: { tokens: Tokens }) {
  const page: CSSProperties = {
    ...previewVars(tokens),
    background: 'var(--fk-color-page-bg)',
  }

  return (
    <div
      role='group'
      aria-label='Preview of the sign-in page'
      style={page}
      className='flex w-full items-center justify-center overflow-auto rounded-lg border p-8'
    >
      <div style={widget} className='w-full max-w-sm' inert>
        <div style={column('var(--fk-spacing-section-gap)')}>
          <div className='flex flex-col items-center gap-2 text-center'>
            <div className='h-9 w-9 rounded bg-black/10' aria-hidden />
            <h3 style={title}>Welcome back</h3>
            <p style={subtitle}>Sign in to continue to your account.</p>
          </div>

          <div style={errorLine} data-preview='error'>
            <CircleAlert size={16} aria-hidden />
            <span>Wrong email or password.</span>
          </div>

          <div style={column('var(--fk-spacing-field-gap)')}>
            <div className='flex flex-col gap-1.5'>
              <span style={label}>Email</span>
              <input
                style={input}
                className='h-10 px-3 text-sm outline-none'
                placeholder='you@example.com'
                readOnly
                tabIndex={-1}
              />
            </div>
            <div className='flex flex-col gap-1.5'>
              <span style={label}>Password</span>
              <input
                style={input}
                className='h-10 px-3 text-sm outline-none'
                type='password'
                value='••••••••'
                readOnly
                tabIndex={-1}
              />
            </div>
            <button type='button' style={primary} className='h-10 w-full px-4' tabIndex={-1}>
              Sign in
            </button>
            <button type='button' style={secondary} className='h-10 w-full px-4' tabIndex={-1}>
              Use another account
            </button>
          </div>

          <div style={divider}>
            <span style={rule} />
            <span>or</span>
            <span style={rule} />
          </div>

          <div className='flex flex-col gap-2'>
            {['Google', 'GitHub'].map((provider) => (
              <button
                key={provider}
                type='button'
                style={social}
                className='flex h-10 w-full items-center justify-center gap-2 px-3'
                tabIndex={-1}
              >
                Continue with {provider}
              </button>
            ))}
            <button
              type='button'
              style={passkey}
              className='flex h-10 w-full items-center justify-center gap-2 px-3'
              tabIndex={-1}
            >
              <KeyRound size={16} aria-hidden />
              Sign in with a passkey
            </button>
            <button
              type='button'
              style={magicLink}
              className='flex h-10 w-full items-center justify-center gap-2 px-3'
              tabIndex={-1}
            >
              <Wand2 size={16} aria-hidden />
              Email me a magic link
            </button>
          </div>

          <div className='flex justify-center'>
            <span style={link}>Forgot your password?</span>
          </div>
        </div>
      </div>
    </div>
  )
}
