import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import type { Speaker } from '../audio/speaker'
import type { Session } from '../session/session'
import type { SessionIntent } from '../settings/protocol'
import type { RemoteSettings } from '../settings/settings'
import { endingMessage } from './messages'
import { ApprovalPanel, ProgressPanel, SecretPanel, WindowsSessionPanel } from './panels'
import { ProblemPanel } from './problem-panel'
import type { BarFrame } from './remote-bar'
import { RemoteView } from './remote-view'
import { useStore } from './use-remote'
import type { FrozenFrame } from './use-screen'
import { Waiting } from './waiting'

export interface Actions {
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
  freeze: FrozenFrame
}

export function SessionStep({ session, ...actions }: { session: Session } & Actions) {
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
