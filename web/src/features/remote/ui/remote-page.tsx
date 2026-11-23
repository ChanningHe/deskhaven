import { useContext } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import type { RemotePhase } from '../connect/controller'
import { CLOSED_BY_USER, type Session } from '../session/session'
import { RemoteDepsContext } from './deps-context'
import { endingMessage, failureMessage } from './messages'
import {
  ApprovalPanel,
  ProblemPanel,
  ProgressPanel,
  SecretPanel,
  WindowsSessionPanel,
} from './panels'
import { RemoteBar, type BarFrame, type BarStatus } from './remote-bar'
import { RemoteView } from './remote-view'
import { useRemote, useStore } from './use-remote'

interface Actions {
  frame: BarFrame
  onReconnect: () => void
  onDisconnect: () => void
}

/** A step before (or after) the screen: the bar, and one panel in the middle of the stage. */
function Waiting({
  frame,
  status,
  children,
}: {
  frame: BarFrame
  status: BarStatus
  children: ReactNode
}) {
  return (
    <>
      <RemoteBar {...frame} status={status} />
      <div className="flex min-h-0 flex-1 overflow-y-auto bg-muted/40 px-4 py-8 sm:pb-[12vh] dark:bg-background">
        <div className="m-auto w-full max-w-md">{children}</div>
      </div>
    </>
  )
}

function SessionStep({
  session,
  frame,
  onReconnect,
  onDisconnect,
}: { session: Session } & Actions) {
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
      return <RemoteView session={session} frame={frame} onDisconnect={onDisconnect} />
    case 'closed':
      return (
        <Waiting frame={frame} status="ended">
          <ProblemPanel
            title={t('remote.ended.title')}
            message={endingMessage(state.reason, t)}
            onRetry={onReconnect}
            retryLabel={t('remote.ended.reconnect')}
          />
        </Waiting>
      )
  }
}

function PhaseStep({ phase, frame, onReconnect, onDisconnect }: { phase: RemotePhase } & Actions) {
  const { t } = useTranslation()
  switch (phase.kind) {
    case 'idle':
      return (
        <Waiting frame={frame} status="ended">
          <ProblemPanel
            title={t('remote.ended.title')}
            message={endingMessage(CLOSED_BY_USER, t)}
            onRetry={onReconnect}
            retryLabel={t('remote.ended.reconnect')}
          />
        </Waiting>
      )
    case 'opening':
      return (
        <Waiting frame={frame} status="connecting">
          <ProgressPanel text={t('remote.opening')} onCancel={onDisconnect} />
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
          />
        </Waiting>
      )
    case 'session':
      return (
        <SessionStep
          session={phase.session}
          frame={frame}
          onReconnect={onReconnect}
          onDisconnect={onDisconnect}
        />
      )
  }
}

/**
 * Remote control of one device from the browser, over WebRTC or the relay: a full-viewport workspace
 * with one thin bar, outside the console's sidebar and top bar.
 */
export function RemotePage({ deviceId, back }: { deviceId: string; back?: ReactNode }) {
  const { phase, transport, reconnect, disconnect } = useRemote(
    deviceId,
    useContext(RemoteDepsContext),
  )
  return (
    <div className="flex h-svh flex-col overflow-hidden bg-background text-foreground">
      <PhaseStep
        phase={phase}
        frame={{ deviceId, back, transport: transport === 'connecting' ? undefined : transport }}
        onReconnect={reconnect}
        onDisconnect={disconnect}
      />
    </div>
  )
}
