import { createFileRoute, useRouterState } from '@tanstack/react-router'

import { requireSession } from '@/features/auth/session'
import { BackLink, LeaveLink } from '@/features/remote/ui/back-link'
import { RemotePage } from '@/features/remote/ui/remote-page'

function RemoteRoute() {
  const { deviceId } = Route.useParams()
  const autoConnect = useRouterState({ select: (s) => s.location.state.remoteConnect === true })
  return (
    <RemotePage
      deviceId={deviceId}
      autoConnect={autoConnect}
      back={<BackLink />}
      leave={<LeaveLink />}
    />
  )
}

// Outside the `_app` layout on purpose (full viewport, no shell), so it takes the session guard here.
export const Route = createFileRoute('/remote/$deviceId')({
  beforeLoad: async ({ context, location }) => ({
    me: await requireSession(context.queryClient, location.href),
  }),
  component: RemoteRoute,
})
