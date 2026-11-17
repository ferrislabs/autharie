import { useHoldsPlatformRight } from '@/domain/organisations/hooks/use-is-operator'
import { useGetOfferOverrides, useSetOfferOverride } from '../../hooks/use-offer-overrides'
import { offerOverrideOf } from '../../offer-override'
import type { OfferChoice, OfferName } from '../../types'
import { OfferOverrides } from '../ui/offer-overrides'

export default function OfferOverridesFeature({ organisationId }: { organisationId: string }) {
  const offers = useGetOfferOverrides(organisationId)
  const setOffer = useSetOfferOverride()
  const canChange = useHoldsPlatformRight('act_on_tenant')

  const choose = (offer: OfferName, choice: OfferChoice) => {
    setOffer.mutate({
      path: { organisation_id: organisationId, offer },
      body: { open: offerOverrideOf(choice) },
    })
  }

  return (
    <OfferOverrides
      offers={offers.data?.data ?? []}
      isLoading={offers.isLoading}
      canChange={canChange}
      pending={setOffer.isPending ? (setOffer.variables?.path.offer as OfferName) : undefined}
      refusal={
        setOffer.isError
          ? setOffer.error instanceof Error
            ? setOffer.error.message
            : 'The offer could not be changed.'
          : undefined
      }
      onChoose={choose}
    />
  )
}
