import { createFileRoute } from '@tanstack/react-router'

import { requireSession } from '@/features/auth/session'
import { BackLink, LeaveLink } from '@/features/remote/ui/back-link'
import { RemotePage } from '@/features/remote/ui/remote-page'

function RemoteRoute() {
  const { deviceId } = Route.useParams()
  return <RemotePage deviceId={deviceId} back={<BackLink />} leave={<LeaveLink />} />
}

// Outside the `_app` layout on purpose: the remote screen is a full-viewport workspace with
// no sidebar or top bar, so it takes the same session guard here. Every signed-in role may
// open it: the server checks the device's ownership and the web_client.enabled switch. The
// page and the protocol code load only with this route.
export const Route = createFileRoute('/remote/$deviceId')({
  beforeLoad: async ({ context, location }) => ({
    me: await requireSession(context.queryClient, location.href),
  }),
  component: RemoteRoute,
})
