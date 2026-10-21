import { Outlet } from '@tanstack/react-router'
import { Page } from './page'

export function DeploymentTabLayout() {
  return (
    <Page>
      <Outlet />
    </Page>
  )
}
