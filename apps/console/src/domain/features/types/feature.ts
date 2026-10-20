import type { Schemas } from '@/api/api.client'

export type Feature = Schemas.Feature
export type FeatureOverview = Schemas.FeatureView
export type TenantFeature = Schemas.TenantFeatureView

export type OverrideChoice = 'default' | 'on' | 'off'
