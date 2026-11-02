import { createFileRoute } from '@tanstack/react-router'

import { MePage } from '@/features/me/components/me-page'

// Every signed-in role may open its own page (permission `self`).
export const Route = createFileRoute('/_app/me')({ component: MePage })
