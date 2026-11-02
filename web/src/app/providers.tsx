import { CSPProvider } from '@base-ui/react/csp-provider'
import { QueryClientProvider } from '@tanstack/react-query'
import type { QueryClient } from '@tanstack/react-query'
import type { ReactNode } from 'react'

import { TooltipProvider } from '@/components/ui/tooltip'
import { ThemeProvider } from '@/lib/theme/theme-provider'

interface AppProvidersProps {
  queryClient: QueryClient
  children: ReactNode
}

export function AppProviders({ queryClient, children }: AppProvidersProps) {
  return (
    // The strict CSP (style-src 'self') blocks inline <style> elements; the one
    // rule Base UI would inject lives in index.css instead.
    <CSPProvider disableStyleElements>
      <QueryClientProvider client={queryClient}>
        <ThemeProvider>
          <TooltipProvider>{children}</TooltipProvider>
        </ThemeProvider>
      </QueryClientProvider>
    </CSPProvider>
  )
}
