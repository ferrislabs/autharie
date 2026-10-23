import { COLOR_KEYS, effectiveTokens, type Shadow, type Tokens } from './theme-tokens'

const SHADOW_CSS: Record<Shadow, string> = {
  none: 'none',
  small: '0 1px 2px rgba(0,0,0,0.05), 0 4px 8px -2px rgba(0,0,0,0.05)',
  large: '0 4px 14px rgba(0,0,0,0.06), 0 12px 32px -4px rgba(0,0,0,0.08)',
}

const COLOR_VARS: Record<(typeof COLOR_KEYS)[number], string> = {
  primaryButton: '--fk-color-primary-button',
  primaryButtonLabel: '--fk-color-primary-button-label',
  secondaryButton: '--fk-color-secondary-button',
  secondaryButtonLabel: '--fk-color-secondary-button-label',
  socialButtonBackground: '--fk-color-social-button-bg',
  socialButtonLabel: '--fk-color-social-button-label',
  socialButtonBorder: '--fk-color-social-button-border',
  magicLinkButtonBackground: '--fk-color-magic-link-button-bg',
  magicLinkButtonLabel: '--fk-color-magic-link-button-label',
  magicLinkButtonBorder: '--fk-color-magic-link-button-border',
  passkeyButtonBackground: '--fk-color-passkey-button-bg',
  passkeyButtonLabel: '--fk-color-passkey-button-label',
  passkeyButtonBorder: '--fk-color-passkey-button-border',
  widgetBackground: '--fk-color-widget-bg',
  pageBackground: '--fk-color-page-bg',
  bodyText: '--fk-color-body-text',
  links: '--fk-color-links',
  error: '--fk-color-error',
}

function px(value: number): string {
  return `${value}px`
}

export function previewVars(tokens: Tokens): Record<string, string> {
  const { colors, fonts, borders, spacing } = effectiveTokens(tokens)
  const size = (pct: number) => px((pct / 100) * fonts.baseSize)

  const vars: Record<string, string> = {}
  for (const key of COLOR_KEYS) vars[COLOR_VARS[key]] = colors[key].trim().toLowerCase()

  return {
    ...vars,
    '--fk-font-base-size': px(fonts.baseSize),
    '--fk-font-title-size': size(fonts.title.sizePct),
    '--fk-font-title-weight': String(fonts.title.weight),
    '--fk-font-subtitle-size': size(fonts.subtitle.sizePct),
    '--fk-font-subtitle-weight': String(fonts.subtitle.weight),
    '--fk-font-body-size': size(fonts.body.sizePct),
    '--fk-font-body-weight': String(fonts.body.weight),
    '--fk-font-button-size': size(fonts.buttons.sizePct),
    '--fk-font-button-weight': String(fonts.buttons.weight),
    '--fk-font-input-label-size': size(fonts.inputLabels.sizePct),
    '--fk-font-input-label-weight': String(fonts.inputLabels.weight),
    '--fk-font-link-size': size(fonts.links.sizePct),
    '--fk-font-link-weight': String(fonts.links.weight),
    '--fk-font-link-decoration': fonts.links.style === 'underline' ? 'underline' : 'none',
    '--fk-radius-button': px(borders.buttonRadius),
    '--fk-border-button': px(borders.buttonBorderWeight),
    '--fk-border-social-button': px(borders.socialButtonBorderWeight),
    '--fk-border-magic-link-button': px(borders.magicLinkButtonBorderWeight),
    '--fk-border-passkey-button': px(borders.passkeyButtonBorderWeight),
    '--fk-radius-input': px(borders.inputRadius),
    '--fk-border-input': px(borders.inputBorderWeight),
    '--fk-radius-widget': px(borders.widgetRadius),
    '--fk-border-widget': px(borders.widgetBorderWeight),
    '--fk-shadow-widget': SHADOW_CSS[borders.widgetShadow],
    '--fk-spacing-widget-padding': px(spacing.widgetPadding),
    '--fk-spacing-field-gap': px(spacing.fieldGap),
    '--fk-spacing-section-gap': px(spacing.sectionGap),
  }
}
