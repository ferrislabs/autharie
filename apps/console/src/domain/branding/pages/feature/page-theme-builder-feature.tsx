import { lazy, Suspense, useState } from 'react'
import { useBlocker, useNavigate, useParams } from '@tanstack/react-router'
import { useGetDeploymentFeatures } from '@/api/billing.api'
import { useActivateTheme, useGetThemes, useUpdateTheme } from '@/api/themes.api'
import { Skeleton } from '@/components/ui/skeleton'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { CAN } from '@/domain/organisations/permissions'
import { isBrandingOpen } from '../../branding-plan'
import { isNotFerriskey } from '../../smtp-state'
import { toTokens, type ThemeDraft } from '../../theme-tokens'
import { useBrandingRealm } from '../../use-branding-realm'
import { useBrandingScope } from '../../use-branding-scope'
import { ConfirmDialog } from '../ui/components/confirm-dialog'

const PageThemeBuilder = lazy(() =>
  import('../ui/page-theme-builder').then((module) => ({ default: module.PageThemeBuilder }))
)

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : 'Something went wrong. Try again.'
}

export default function PageThemeBuilderFeature() {
  const scope = useBrandingScope()
  const [realm] = useBrandingRealm()
  const { themeId } = useParams({ strict: false }) as { themeId?: string }
  const { can } = useMyPermissions()
  const navigate = useNavigate()
  const deploymentPath = useDeploymentPath()

  const features = useGetDeploymentFeatures(scope.organisationId, scope.deploymentId)
  const themes = useGetThemes(scope, realm)
  const update = useUpdateTheme()
  const activate = useActivateTheme()

  const [dirty, setDirty] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string>()

  const blocker = useBlocker({
    shouldBlockFn: () => dirty,
    enableBeforeUnload: dirty,
    withResolver: true,
  })

  const theme = themes.data?.data.find((entry) => entry.id === themeId) ?? null
  const unsupported = isNotFerriskey(themes.error)
  const path =
    scope.organisationId && scope.deploymentId && themeId
      ? {
          organisation_id: scope.organisationId,
          deployment_id: scope.deploymentId,
          realm,
          theme_id: themeId,
        }
      : null

  const onSave = async (draft: ThemeDraft, withActivation: boolean) => {
    if (!path || saving) return

    setSaving(true)
    setSaveError(undefined)
    try {
      await update.mutateAsync({
        path,
        body: { name: draft.name.trim(), tokens: toTokens(draft.tokens) },
      })
      if (withActivation) await activate.mutateAsync({ path })
    } catch (error) {
      setSaveError(messageOf(error))
    } finally {
      setSaving(false)
    }
  }

  const onBack = () =>
    void navigate({ to: deploymentPath('/branding/universal-login'), search: { realm } })

  return (
    <>
      <Suspense fallback={<Skeleton className='h-96 w-full' />}>
        <PageThemeBuilder
          isLoading={features.isLoading || themes.isLoading}
          unsupported={unsupported}
          error={themes.error && !unsupported ? themes.error.message : undefined}
          theme={theme}
          state={themes.data?.state ?? null}
          planOpen={isBrandingOpen(features.data?.features)}
          tier={features.data?.tier}
          canManage={can(CAN.manageInstances)}
          isSaving={saving}
          saveError={saveError}
          onSave={onSave}
          onBack={onBack}
          onDirtyChange={setDirty}
        />
      </Suspense>

      <ConfirmDialog
        open={blocker.status === 'blocked'}
        onOpenChange={(open) => {
          if (!open) blocker.reset?.()
        }}
        title='Discard your changes?'
        description='This theme has changes that are not saved. Leaving now loses them.'
        confirmLabel='Discard changes'
        cancelLabel='Keep editing'
        destructive
        onConfirm={() => blocker.proceed?.()}
      />
    </>
  )
}
