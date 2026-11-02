import './index.css'
import '@/i18n'

import { RouterProvider } from '@tanstack/react-router'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { createApp } from '@/app/create-app'
import { AppProviders } from '@/app/providers'

const { queryClient, router } = createApp()

const rootElement = document.getElementById('root')
if (!rootElement) throw new Error('missing #root element in index.html')

createRoot(rootElement).render(
  <StrictMode>
    <AppProviders queryClient={queryClient}>
      <RouterProvider router={router} />
    </AppProviders>
  </StrictMode>,
)
