import { useCallback, useEffect, useMemo, useSyncExternalStore } from 'react'

import { Speaker } from '../audio/speaker'
import {
  RemoteController,
  type ConnectDeps,
  type RemotePhase,
  type TransportChoice,
} from '../connect/controller'
import type { Store } from '../session/store'
import type { SessionIntent } from '../settings/protocol'
import type { RemoteSettings } from '../settings/settings'

export function useStore<T>(store: Store<T>): T {
  const subscribe = useCallback((listener: () => void) => store.subscribe(listener), [store])
  const get = useCallback(() => store.get(), [store])
  return useSyncExternalStore(subscribe, get)
}

export interface Remote {
  phase: RemotePhase
  transport: TransportChoice | null
  settings: RemoteSettings
  updateSettings: (patch: Partial<RemoteSettings>) => void
  intent: SessionIntent
  updateIntent: (patch: Partial<SessionIntent>) => void
  speaker: Speaker
  reconnect: () => void
  disconnect: () => void
}

export function useRemote(deviceId: string, deps: ConnectDeps, started: boolean): Remote {
  const controller = useMemo(() => new RemoteController(deviceId, deps), [deviceId, deps])

  useEffect(() => {
    if (!started) return
    controller.connect()
    const leave = () => {
      controller.disconnect()
    }
    window.addEventListener('pagehide', leave)
    return () => {
      window.removeEventListener('pagehide', leave)
      controller.disconnect()
    }
  }, [controller, started])

  const speaker = useMemo(() => new Speaker(deps.audio), [deps])
  const settings = useStore(controller.settings)
  const { supported, running } = useStore(speaker.view)
  useEffect(() => {
    void speaker.probe()
    return () => {
      speaker.stop()
    }
  }, [speaker])
  useEffect(() => {
    if (settings.audio && supported) speaker.prepare()
    else speaker.stop()
  }, [speaker, settings.audio, supported])
  useEffect(() => {
    controller.setAudioRunning(running)
  }, [controller, running])

  return {
    phase: useStore(controller.phase),
    transport: useStore(controller.transport),
    settings,
    updateSettings: (patch) => {
      controller.updateSettings(patch)
    },
    intent: useStore(controller.intent),
    updateIntent: (patch) => {
      controller.updateIntent(patch)
    },
    speaker,
    reconnect: () => {
      controller.connect()
    },
    disconnect: () => {
      controller.disconnect()
    },
  }
}
