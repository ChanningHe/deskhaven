import { render } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactElement } from 'react'

import { AppProviders } from '@/app/providers'
import { createQueryClient } from '@/app/query-client'

/** Renders a component with the app's providers (no router). */
export function renderWithProviders(ui: ReactElement) {
  const user = userEvent.setup()
  const result = render(<AppProviders queryClient={createQueryClient()}>{ui}</AppProviders>)
  return { ...result, user }
}
