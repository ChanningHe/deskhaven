import { createFileRoute } from '@tanstack/react-router'

import { RemotePage } from '@/features/remote/ui/remote-page'

function RemoteRoute() {
  const { deviceId } = Route.useParams()
  return <RemotePage deviceId={deviceId} />
}

// Every signed-in role may open it: the server checks the device's ownership and the
// web_client.enabled switch. The page and the protocol code load only with this route.
export const Route = createFileRoute('/_app/remote/$deviceId')({ component: RemoteRoute })
