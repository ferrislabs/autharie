import type { Schemas } from '@/api/api.client'

export type OfferName = Schemas.Offer

export type OfferOverrideView = Schemas.TenantOfferView

export type OfferOverridesResponse = Schemas.TenantOffersResponse

export type OfferOverrideResponse = Schemas.TenantOfferResponse

export type OfferChoice = 'default' | 'open' | 'closed'
