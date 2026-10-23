import { Button } from '@/components/ui/button'
import { PANELS, resetPanel, type PanelId } from '../../../theme-panels'
import { LINK_STYLES, RANGES, SHADOWS } from '../../../theme-tokens'
import {
  ChoiceControl,
  ColorControl,
  NumberControl,
  TextControl,
  type PanelContext,
} from './theme-controls'

const SHADOW_OPTIONS = SHADOWS.map((value) => ({
  value,
  label: value[0].toUpperCase() + value.slice(1),
}))
const LINK_STYLE_OPTIONS = LINK_STYLES.map((value) => ({
  value,
  label: value === 'normal' ? 'Normal' : 'Underlined',
}))

function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className='space-y-2.5 border-t pt-3 first:border-t-0 first:pt-0'>
      <h3 className='text-xs font-semibold uppercase tracking-wide text-muted-foreground'>
        {title}
      </h3>
      {children}
    </div>
  )
}

function Colour({ label, path, ctx }: { label: string; path: string; ctx: PanelContext }) {
  return <ColorControl label={label} path={`colors.${path}`} ctx={ctx} />
}

function Size({ label, path, ctx }: { label: string; path: string; ctx: PanelContext }) {
  return <NumberControl label={label} path={path} ctx={ctx} {...RANGES.border} unit='px' />
}

function FontStyle({
  title,
  path,
  ctx,
  children,
}: {
  title: string
  path: string
  ctx: PanelContext
  children?: React.ReactNode
}) {
  return (
    <Group title={title}>
      <NumberControl label='Weight' path={`${path}.weight`} ctx={ctx} {...RANGES.weight} />
      <NumberControl label='Size' path={`${path}.sizePct`} ctx={ctx} {...RANGES.sizePct} unit='%' />
      {children}
    </Group>
  )
}

function ColorsPanel({ ctx }: { ctx: PanelContext }) {
  return (
    <div className='space-y-2.5'>
      <Colour label='Primary button' path='primaryButton' ctx={ctx} />
      <Colour label='Primary button label' path='primaryButtonLabel' ctx={ctx} />
      <Colour label='Links' path='links' ctx={ctx} />
      <Colour label='Page background' path='pageBackground' ctx={ctx} />
      <Colour label='Card background' path='widgetBackground' ctx={ctx} />
      <Colour label='Text' path='bodyText' ctx={ctx} />
      <Colour label='Error' path='error' ctx={ctx} />
    </div>
  )
}

function TypographyPanel({ ctx }: { ctx: PanelContext }) {
  return (
    <div className='space-y-3'>
      <Group title='Font'>
        <TextControl
          label='Custom font URL'
          path='fonts.url'
          ctx={ctx}
          placeholder='https://fonts.googleapis.com/…'
        />
        <NumberControl
          label='Base size'
          path='fonts.baseSize'
          ctx={ctx}
          {...RANGES.baseSize}
          unit='px'
        />
        <p className='text-xs text-muted-foreground'>
          The font is loaded by the login page itself; the preview keeps the console’s font.
        </p>
      </Group>
      <FontStyle title='Title' path='fonts.title' ctx={ctx} />
      <FontStyle title='Subtitle' path='fonts.subtitle' ctx={ctx} />
      <FontStyle title='Body' path='fonts.body' ctx={ctx} />
      <FontStyle title='Links' path='fonts.links' ctx={ctx}>
        <ChoiceControl
          label='Style'
          path='fonts.links.style'
          ctx={ctx}
          options={LINK_STYLE_OPTIONS}
        />
      </FontStyle>
    </div>
  )
}

function ButtonsPanel({ ctx }: { ctx: PanelContext }) {
  return (
    <div className='space-y-3'>
      <p className='text-xs text-muted-foreground'>The primary button’s colors are in Colors.</p>
      <Group title='Secondary'>
        <Colour label='Background' path='secondaryButton' ctx={ctx} />
        <Colour label='Label' path='secondaryButtonLabel' ctx={ctx} />
      </Group>
      <Group title='Social sign-in'>
        <Colour label='Background' path='socialButtonBackground' ctx={ctx} />
        <Colour label='Label' path='socialButtonLabel' ctx={ctx} />
        <Colour label='Border' path='socialButtonBorder' ctx={ctx} />
        <Size label='Border width' path='borders.socialButtonBorderWeight' ctx={ctx} />
      </Group>
      <Group title='Magic link'>
        <Colour label='Background' path='magicLinkButtonBackground' ctx={ctx} />
        <Colour label='Label' path='magicLinkButtonLabel' ctx={ctx} />
        <Colour label='Border' path='magicLinkButtonBorder' ctx={ctx} />
        <Size label='Border width' path='borders.magicLinkButtonBorderWeight' ctx={ctx} />
      </Group>
      <Group title='Passkey'>
        <Colour label='Background' path='passkeyButtonBackground' ctx={ctx} />
        <Colour label='Label' path='passkeyButtonLabel' ctx={ctx} />
        <Colour label='Border' path='passkeyButtonBorder' ctx={ctx} />
        <Size label='Border width' path='borders.passkeyButtonBorderWeight' ctx={ctx} />
      </Group>
      <Group title='Shape'>
        <Size label='Radius' path='borders.buttonRadius' ctx={ctx} />
        <Size label='Border width' path='borders.buttonBorderWeight' ctx={ctx} />
      </Group>
      <FontStyle title='Text' path='fonts.buttons' ctx={ctx} />
    </div>
  )
}

function InputsPanel({ ctx }: { ctx: PanelContext }) {
  return (
    <div className='space-y-3'>
      <Group title='Shape'>
        <Size label='Radius' path='borders.inputRadius' ctx={ctx} />
        <Size label='Border width' path='borders.inputBorderWeight' ctx={ctx} />
      </Group>
      <FontStyle title='Labels' path='fonts.inputLabels' ctx={ctx} />
    </div>
  )
}

function WidgetPanel({ ctx }: { ctx: PanelContext }) {
  return (
    <div className='space-y-2.5'>
      <Size label='Radius' path='borders.widgetRadius' ctx={ctx} />
      <Size label='Border width' path='borders.widgetBorderWeight' ctx={ctx} />
      <ChoiceControl
        label='Shadow'
        path='borders.widgetShadow'
        ctx={ctx}
        options={SHADOW_OPTIONS}
      />
      <NumberControl
        label='Padding'
        path='spacing.widgetPadding'
        ctx={ctx}
        {...RANGES.spacing}
        unit='px'
      />
    </div>
  )
}

function PagePanel({ ctx }: { ctx: PanelContext }) {
  return (
    <div className='space-y-2.5'>
      <NumberControl
        label='Gap between fields'
        path='spacing.fieldGap'
        ctx={ctx}
        {...RANGES.spacing}
        unit='px'
      />
      <NumberControl
        label='Gap between sections'
        path='spacing.sectionGap'
        ctx={ctx}
        {...RANGES.spacing}
        unit='px'
      />
    </div>
  )
}

const BODIES: Record<PanelId, (props: { ctx: PanelContext }) => React.ReactNode> = {
  colors: ColorsPanel,
  typography: TypographyPanel,
  buttons: ButtonsPanel,
  inputs: InputsPanel,
  widget: WidgetPanel,
  page: PagePanel,
}

export function PanelBody({
  id,
  ctx,
  onReset,
}: {
  id: PanelId
  ctx: PanelContext
  onReset: (tokens: PanelContext['tokens']) => void
}) {
  const Body = BODIES[id]
  const panel = PANELS.find((entry) => entry.id === id)

  return (
    <div className='space-y-4'>
      <p className='text-sm text-muted-foreground'>{panel?.description}</p>
      <Body ctx={ctx} />
      {!ctx.readOnly && (
        <Button
          type='button'
          variant='outline'
          size='sm'
          onClick={() => onReset(resetPanel(ctx.tokens, id))}
        >
          Reset {panel?.label.toLowerCase()} to defaults
        </Button>
      )}
    </div>
  )
}
