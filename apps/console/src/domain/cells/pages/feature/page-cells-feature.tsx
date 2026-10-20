import {
  useCreateCell,
  useDrainCell,
  useGetCells,
  useGetCellsSettings,
  useRetireCell,
  useUpdateCellsSettings,
} from '@/api/cells.api'
import { useGetTenants } from '@/api/platform.api'
import { useHoldsPlatformRight } from '@/domain/organisations/hooks/use-is-operator'
import { PageCells } from '../ui/page-cells'

const message = (error: unknown) => (error instanceof Error ? error.message : undefined)

export default function PageCellsFeature() {
  const cells = useGetCells()
  const settings = useGetCellsSettings()
  const organisations = useGetTenants()
  const canOperate = useHoldsPlatformRight('operate_fleet')

  const update = useUpdateCellsSettings()
  const create = useCreateCell()
  const drain = useDrainCell()
  const retire = useRetireCell()

  return (
    <PageCells
      cells={cells.data?.data ?? []}
      settings={settings.data}
      organisations={organisations.data?.data ?? []}
      isLoading={cells.isLoading || settings.isLoading}
      canOperate={canOperate}
      onSaveOwner={(owner) => {
        update.reset()
        update.mutate({ body: { organisation_id: owner } })
      }}
      isSavingOwner={update.isPending}
      ownerRefusal={message(update.error)}
      onOpenCell={(region, onDone) => {
        create.mutate({ body: { region } }, { onSuccess: onDone })
      }}
      isOpening={create.isPending}
      openRefusal={message(create.error)}
      onDrain={(cellId) => {
        retire.reset()
        drain.mutate({ path: { cell_id: cellId } })
      }}
      onRetire={(cellId) => {
        drain.reset()
        retire.mutate({ path: { cell_id: cellId } })
      }}
      pendingCellId={drain.variables?.path.cell_id ?? retire.variables?.path.cell_id}
      actionRefusal={message(drain.error) ?? message(retire.error)}
    />
  )
}
