import { useContext } from 'react'
import { useTranslation } from 'react-i18next'

import { PageHeader } from '@/components/common/page-header'

import type { RemotePhase } from '../connect/controller'
import { CLOSED_BY_USER, type Session } from '../session/session'
import { RemoteDepsContext } from './deps-context'
import { endingMessage, failureMessage } from './messages'
import { ApprovalPanel, ProblemPanel, ProgressPanel, SecretPanel } from './panels'
import { RemoteView } from './remote-view'
import { useRemote, useStore } from './use-remote'

interface Actions {
  onReconnect: () => void
  onDisconnect: () => void
}

function SessionStep({ session, onReconnect, onDisconnect }: { session: Session } & Actions) {
  const { t } = useTranslation()
  const state = useStore(session.state)
  switch (state.kind) {
    case 'connecting':
      return <ProgressPanel text={t('remote.connecting')} onCancel={onDisconnect} />
    case 'need_password':
    case 'wrong_password':
      return (
        <SecretPanel
          key="password"
          kind="password"
          wrong={state.kind === 'wrong_password'}
          onSubmit={(password) => void session.login(password)}
          onCancel={onDisconnect}
        />
      )
    case 'need_2fa':
      return (
        <SecretPanel
          key="twoFactor"
          kind="twoFactor"
          wrong={state.wrongCode}
          onSubmit={(code) => {
            session.submit2fa(code)
          }}
          onCancel={onDisconnect}
        />
      )
    case 'awaiting_approval':
      return <ApprovalPanel onCancel={onDisconnect} />
    case 'active':
      return <RemoteView session={session} onDisconnect={onDisconnect} />
    case 'closed':
      return (
        <ProblemPanel
          title={t('remote.ended.title')}
          message={endingMessage(state.reason, t)}
          onRetry={onReconnect}
          retryLabel={t('remote.ended.reconnect')}
        />
      )
  }
}

function PhaseStep({ phase, onReconnect, onDisconnect }: { phase: RemotePhase } & Actions) {
  const { t } = useTranslation()
  switch (phase.kind) {
    case 'idle':
      return (
        <ProblemPanel
          title={t('remote.ended.title')}
          message={endingMessage(CLOSED_BY_USER, t)}
          onRetry={onReconnect}
          retryLabel={t('remote.ended.reconnect')}
        />
      )
    case 'opening':
      return <ProgressPanel text={t('remote.opening')} onCancel={onDisconnect} />
    case 'failed':
      return (
        <ProblemPanel
          title={t('remote.errors.title')}
          message={failureMessage(phase.failure, t)}
          onRetry={onReconnect}
          retryLabel={t('remote.errors.retry')}
        />
      )
    case 'session':
      return (
        <SessionStep
          session={phase.session}
          onReconnect={onReconnect}
          onDisconnect={onDisconnect}
        />
      )
  }
}

/** Remote control of one device from the browser, over the relay. */
export function RemotePage({ deviceId }: { deviceId: string }) {
  const { t } = useTranslation()
  const { phase, reconnect, disconnect } = useRemote(deviceId, useContext(RemoteDepsContext))
  return (
    <div className="space-y-4">
      <PageHeader title={t('remote.title')} description={t('remote.device', { id: deviceId })} />
      <PhaseStep phase={phase} onReconnect={reconnect} onDisconnect={disconnect} />
    </div>
  )
}
