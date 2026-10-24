import { useEffect, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import type { Schemas } from '@/api/api.client'
import { useGetDeploymentFeatures } from '@/api/billing.api'
import { useGetIamSettings } from '@/api/iam-settings.api'
import { useGetBrandingRealms } from '@/api/smtp.api'
import {
  useActivateTheme,
  useCreateTheme,
  useDeactivateTheme,
  useDeleteTheme,
  useGetThemes,
} from '@/api/themes.api'
import { useIdentityRealm, useRealmPath } from '@/domain/identity/use-identity-realm'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { CAN } from '@/domain/organisations/permissions'
import { isBrandingOpen } from '../../branding-plan'
import { downloadFile } from '../../download-file'
import { isNotFerriskey } from '../../smtp-state'
import { exportFileName, exportTheme, type ThemeFile } from '../../theme-files'
import { availableName, copyName } from '../../theme-names'
import {
  LEGACY_THEME_NAME,
  defaultTokens,
  legacyToTokens,
  toTokens,
  withDefaults,
} from '../../theme-tokens'
import { useBrandingScope } from '../../use-branding-scope'
import { PageUniversalLogin } from '../ui/page-universal-login'

interface RealmPath {
  organisation_id: string
  deployment_id: string
  realm: string
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : 'Something went wrong. Try again.'
}

export default function PageUniversalLoginFeature() {
  const scope = useBrandingScope()
  const realm = useIdentityRealm()
  const { can } = useMyPermissions()
  const navigate = useNavigate()
  const realmPath = useRealmPath()

  const features = useGetDeploymentFeatures(scope.organisationId, scope.deploymentId)
  const themes = useGetThemes(scope, realm)
  const realms = useGetBrandingRealms(scope)
  const settings = useGetIamSettings(scope.organisationId, scope.deploymentId, realm === 'master')
  const create = useCreateTheme()
  const remove = useDeleteTheme()
  const activate = useActivateTheme()
  const deactivate = useDeactivateTheme()

  const [working, setWorking] = useState(false)
  const [actionError, setActionError] = useState<string>()

  // The realm is in the address, so choosing another one is a navigation and
  // this page stays mounted: an error about the last realm is not one about
  // the next.
  useEffect(() => setActionError(undefined), [realm])

  const list = themes.data?.data ?? []
  const unsupported = isNotFerriskey(themes.error) || isNotFerriskey(realms.error)
  const path: RealmPath | null =
    scope.organisationId && scope.deploymentId
      ? { organisation_id: scope.organisationId, deployment_id: scope.deploymentId, realm }
      : null

  const run = async (job: (path: RealmPath) => Promise<unknown>) => {
    if (!path || working) return

    setWorking(true)
    setActionError(undefined)
    try {
      await job(path)
    } catch (error) {
      setActionError(messageOf(error))
    } finally {
      setWorking(false)
    }
  }

  const open = (themeId: string) =>
    void navigate({ to: realmPath(`/branding/universal-login/${themeId}`) })

  const names = list.map((theme) => theme.name)

  const onCreate = (name: string) =>
    run(async (path) => {
      const created = await create.mutateAsync({
        path,
        body: { name, tokens: toTokens(defaultTokens()) },
      })
      open(created.data.id)
    })

  const onImport = (file: ThemeFile) =>
    run((path) =>
      create.mutateAsync({
        path,
        body: { name: availableName(file.name, names), tokens: file.tokens },
      })
    )

  const onDuplicate = (theme: Schemas.ThemeView) =>
    run((path) =>
      create.mutateAsync({
        path,
        body: {
          name: copyName(theme.name, names),
          tokens: toTokens(withDefaults(theme.tokens)),
        },
      })
    )

  const onImportLegacy = () =>
    run(async (path) => {
      const created = await create.mutateAsync({
        path,
        body: {
          name: LEGACY_THEME_NAME,
          tokens: legacyToTokens(settings.data?.data?.branding),
        },
      })
      await activate.mutateAsync({ path: { ...path, theme_id: created.data.id } })
    })

  return (
    <PageUniversalLogin
      isLoading={features.isLoading || themes.isLoading}
      unsupported={unsupported}
      error={themes.error && !unsupported ? themes.error.message : undefined}
      actionError={actionError}
      realm={realm}
      themes={list}
      state={themes.data?.state ?? null}
      planOpen={isBrandingOpen(features.data?.features)}
      tier={features.data?.tier}
      canManage={can(CAN.manageInstances)}
      isBusy={working}
      legacy={settings.data?.data?.branding ?? null}
      onCreate={onCreate}
      onImport={onImport}
      onImportLegacy={onImportLegacy}
      onEdit={(theme) => open(theme.id)}
      onActivate={(theme) =>
        run((path) => activate.mutateAsync({ path: { ...path, theme_id: theme.id } }))
      }
      onDuplicate={onDuplicate}
      onExport={(theme) => downloadFile(exportFileName(theme.name), exportTheme(theme))}
      onDelete={(theme) =>
        run((path) => remove.mutateAsync({ path: { ...path, theme_id: theme.id } }))
      }
      onDeactivate={() => run((path) => deactivate.mutateAsync({ path }))}
    />
  )
}
