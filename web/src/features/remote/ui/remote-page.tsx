import { useContext, useState } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import type { RemotePhase } from '../connect/controller'
import { ConfirmPanel } from './confirm-panel'
import { RemoteDepsContext } from './deps-context'
import { endingMessage, failureMessage } from './messages'
import { ProgressPanel } from './panels'
import { ProblemPanel } from './problem-panel'
import { ReconnectStage } from './reconnect-stage'
import { SessionStep, type Actions } from './session-step'
import { useRemote } from './use-remote'
import { useFrozenFrame } from './use-screen'
import { Waiting } from './waiting'

function PhaseStep({ phase, ...actions }: { phase: RemotePhase | { kind: 'confirm' } } & Actions) {
  const { frame, onReconnect, onDisconnect, leave } = actions
  const { t } = useTranslation()
  switch (phase.kind) {
    case 'confirm':
      return (
        <Waiting frame={frame} status="ended">
          <ConfirmPanel deviceId={frame.deviceId} onConfirm={onReconnect} leave={leave} />
        </Waiting>
      )
    case 'idle':
      return (
        <Waiting frame={frame} status="ended">
          <ProblemPanel
            title={t('remote.ended.title')}
            message={endingMessage({ by: 'user', reason: '' }, t)}
            onRetry={onReconnect}
            retryLabel={t('remote.ended.reconnect')}
            leave={leave}
          />
        </Waiting>
      )
    case 'opening':
      return (
        <Waiting frame={frame} status="connecting">
          <ProgressPanel text={t('remote.opening')} onCancel={onDisconnect} />
        </Waiting>
      )
    case 'reconnecting':
      return actions.freeze.frozen ? (
        <ReconnectStage {...actions} frozen={actions.freeze.frozen} attempt={phase.attempt} />
      ) : (
        <Waiting frame={frame} status="connecting">
          <ProgressPanel
            text={t('remote.reconnecting', { attempt: phase.attempt })}
            onCancel={onDisconnect}
          />
        </Waiting>
      )
    case 'failed':
      return (
        <Waiting frame={frame} status="ended">
          <ProblemPanel
            title={t('remote.errors.title')}
            message={failureMessage(phase.failure, t)}
            onRetry={onReconnect}
            retryLabel={t('remote.errors.retry')}
            leave={leave}
          />
        </Waiting>
      )
    case 'session':
      return <SessionStep session={phase.session} {...actions} />
  }
}

export function RemotePage({
  deviceId,
  autoConnect,
  back,
  leave,
}: {
  deviceId: string
  /** False unless opened from the console's own Connect link: a crafted link must not connect. */
  autoConnect: boolean
  back?: ReactNode
  leave?: ReactNode
}) {
  const [started, setStarted] = useState(autoConnect)
  const {
    phase,
    transport,
    settings,
    updateSettings,
    intent,
    updateIntent,
    speaker,
    reconnect,
    disconnect,
  } = useRemote(deviceId, useContext(RemoteDepsContext), started)
  // Here, not in RemoteView: a reconnect remounts the view and must not turn the overlay off.
  const [stats, setStats] = useState(false)
  const freeze = useFrozenFrame(phase)
  return (
    <div className="flex h-svh flex-col overflow-hidden bg-background text-foreground">
      <PhaseStep
        phase={started ? phase : { kind: 'confirm' }}
        frame={{ deviceId, back, transport: transport ?? undefined }}
        settings={settings}
        onSettings={updateSettings}
        intent={intent}
        onIntent={updateIntent}
        speaker={speaker}
        stats={stats}
        onStats={setStats}
        onReconnect={
          started
            ? reconnect
            : () => {
                setStarted(true)
              }
        }
        onDisconnect={disconnect}
        leave={leave}
        freeze={freeze}
      />
    </div>
  )
}
