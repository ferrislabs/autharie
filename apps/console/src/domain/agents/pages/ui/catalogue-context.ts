import { createContext, useContext } from 'react'
import { BUILTIN_CATALOGUE, type Catalogue } from '../../tool-catalogue'

export const CatalogueContext = createContext<Catalogue>(BUILTIN_CATALOGUE)

export function useCatalogue(): Catalogue {
  return useContext(CatalogueContext)
}
