import { createFileRoute } from '@tanstack/react-router'

import { DevicesPage } from '@/features/devices/components/devices-page'

export const Route = createFileRoute('/_app/devices')({ component: DevicesPage })
