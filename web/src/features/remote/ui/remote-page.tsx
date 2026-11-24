import { useContext, useState } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import type { Speaker } from '../audio/speaker'
import type { RemotePhase } from '../connect/controller'
import type { Session } from '../session/session'
import type { SessionIntent } from '../settings/protocol'
import type { RemoteSettings } from '../settings/settings'
import { RemoteDepsContext } from './deps-context'
import { endingMessage, failureMessage } from './messages'
import { ApprovalPanel, ProgressPanel, SecretPanel, WindowsSessionPanel } from './panels'
import { ProblemPanel } from './problem-panel'
import type { BarFrame } from './remote-bar'
import { RemoteView } from './remote-view'
import { useRemote, useStore } from './use-remote'
import { Waiting } from './waiting'

interface Actions {
  frame: BarFrame
  settings: RemoteSettings
  onSettings: (patch: Partial<RemoteSettings>) => void
  intent: SessionIntent
  onIntent: (patch: Partial<SessionIntent>) => void
  speaker: Speaker
  stats: boolean
  onStats: (on: boolean) => void
  onReconnect: () => void
  onDisconnect: () => void
  leave: ReactNode
}

function SessionStep({ session, ...actions }: { session: Session } & Actions) {
  const { frame, onReconnect, onDisconnect, leave } = actions
  const { t } = useTranslation()
  const state = useStore(session.state)
  switch (state.kind) {
    case 'connecting':
      return (
        <Waiting frame={frame} status="connecting">
          <ProgressPanel text={t('remote.connecting')} onCancel={onDisconnect} />
        </Waiting>
      )
    case 'need_password':
    case 'wrong_password':
      return (
        <Waiting frame={frame} status="connecting">
          <SecretPanel
            key="password"
            kind="password"
            wrong={state.kind === 'wrong_password'}
            onSubmit={(password) => void session.login(password)}
            onCancel={onDisconnect}
          />
        </Waiting>
      )
    case 'need_2fa':
      return (
        <Waiting frame={frame} status="connecting">
          <SecretPanel
            key="twoFactor"
            kind="twoFactor"
            wrong={state.wrongCode}
            onSubmit={(code) => {
              session.submit2fa(code)
            }}
            onCancel={onDisconnect}
          />
        </Waiting>
      )
    case 'awaiting_approval':
      return (
        <Waiting frame={frame} status="connecting">
          <ApprovalPanel onCancel={onDisconnect} />
        </Waiting>
      )
    case 'choose_session':
      return (
        <Waiting frame={frame} status="connecting">
          <WindowsSessionPanel
            choice={state}
            onChoose={(sid) => {
              session.chooseWindowsSession(sid)
            }}
            onCancel={onDisconnect}
          />
        </Waiting>
      )
    case 'active':
      return <RemoteView session={session} {...actions} />
    case 'closed':
      return (
        <Waiting frame={frame} status="ended">
          <ProblemPanel
            title={t('remote.ended.title')}
            message={endingMessage(state.cause, t)}
            onRetry={onReconnect}
            retryLabel={t('remote.ended.reconnect')}
            leave={leave}
          />
        </Waiting>
      )
  }
}

function PhaseStep({ phase, ...actions }: { phase: RemotePhase } & Actions) {
  const { frame, onReconnect, onDisconnect, leave } = actions
  const { t } = useTranslation()
  switch (phase.kind) {
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
      return (
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

/**
 * Remote control of one device from the browser, over WebRTC or the relay: a full-viewport workspace
 * with one thin bar, outside the console's sidebar and top bar.
 */
export function RemotePage({
  deviceId,
  back,
  leave,
}: {
  deviceId: string
  back?: ReactNode
  leave?: ReactNode
}) {
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
  } = useRemote(deviceId, useContext(RemoteDepsContext))
  // Here, not in RemoteView: a reconnect remounts the view and must not turn the overlay off.
  const [stats, setStats] = useState(false)
  return (
    <div className="flex h-svh flex-col overflow-hidden bg-background text-foreground">
      <PhaseStep
        phase={phase}
        frame={{ deviceId, back, transport: transport === 'connecting' ? undefined : transport }}
        settings={settings}
        onSettings={updateSettings}
        intent={intent}
        onIntent={updateIntent}
        speaker={speaker}
        stats={stats}
        onStats={setStats}
        onReconnect={reconnect}
        onDisconnect={disconnect}
        leave={leave}
      />
    </div>
  )
}
