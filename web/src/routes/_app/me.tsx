import { createFileRoute } from '@tanstack/react-router'

import { MePage } from '@/features/me/components/me-page'

export const Route = createFileRoute('/_app/me')({ component: MePage })
