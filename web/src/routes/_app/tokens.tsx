import { createFileRoute } from '@tanstack/react-router'

import { TokensPage } from '@/features/tokens/components/tokens-page'

export const Route = createFileRoute('/_app/tokens')({ component: TokensPage })
