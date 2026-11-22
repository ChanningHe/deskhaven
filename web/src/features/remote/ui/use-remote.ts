import { useCallback, useEffect, useMemo, useSyncExternalStore } from 'react'

import {
  RemoteController,
  type ConnectDeps,
  type RemotePhase,
  type TransportKind,
} from '../connect/controller'
import type { Store } from '../session/store'

export function useStore<T>(store: Store<T>): T {
  const subscribe = useCallback((listener: () => void) => store.subscribe(listener), [store])
  const get = useCallback(() => store.get(), [store])
  return useSyncExternalStore(subscribe, get)
}

export interface Remote {
  phase: RemotePhase
  transport: TransportKind
  reconnect: () => void
  disconnect: () => void
}

/**
 * Connects to `deviceId` while the page is shown. Leaving it (another route, or the tab
 * going away: pagehide) hangs up and closes the server session.
 */
export function useRemote(deviceId: string, deps: ConnectDeps): Remote {
  const controller = useMemo(() => new RemoteController(deviceId, deps), [deviceId, deps])

  useEffect(() => {
    controller.connect()
    const leave = () => {
      controller.disconnect()
    }
    window.addEventListener('pagehide', leave)
    return () => {
      window.removeEventListener('pagehide', leave)
      controller.disconnect()
    }
  }, [controller])

  return {
    phase: useStore(controller.phase),
    transport: useStore(controller.transport),
    reconnect: () => {
      controller.connect()
    },
    disconnect: () => {
      controller.disconnect()
    },
  }
}
