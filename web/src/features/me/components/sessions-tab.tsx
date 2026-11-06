import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  getListMySessionsQueryKey,
  revokeMyOtherSessions,
  revokeMySession,
  useListMySessions,
} from '@/api/generated/me/me'
import type { MySession } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { LOGIN_PATH } from '@/features/auth/session'
import { formatDateTime, nonBlank } from '@/lib/format'

import { Section } from './section'

/** The most recently used sessions in one page: the server caps sessions per user. */
const PAGE = { page_size: 200 }

/** What the other end reported about itself, isolated from the surrounding text. */
function DeviceText({ session }: { session: MySession }) {
  const { t } = useTranslation()
  const name = nonBlank(session.device_name) ?? nonBlank(session.user_agent)
  const detail = [nonBlank(session.device_os), nonBlank(session.rustdesk_id)]
    .filter((part) => part !== undefined)
    .join(' · ')
  if (name === undefined && detail === '') {
    return <span className="text-muted-foreground">{t('me.sessions.unknownDevice')}</span>
  }
  return (
    <div className="min-w-0">
      {name === undefined ? null : <bdi className="break-words">{name}</bdi>}
      {detail === '' ? null : (
        <div className="text-xs text-muted-foreground">
          <bdi>{detail}</bdi>
        </div>
      )}
    </div>
  )
}

function SessionRow({
  session,
  onRevoke,
}: {
  session: MySession
  onRevoke: (session: MySession) => void
}) {
  const { t, i18n } = useTranslation()
  const label = nonBlank(session.device_name) ?? nonBlank(session.user_agent) ?? String(session.id)
  return (
    <TableRow>
      <TableCell>
        <div className="flex flex-wrap items-center gap-1">
          <Badge variant="outline">{t(`me.sessions.kind.${session.kind}`)}</Badge>
          {session.current ? <Badge>{t('me.sessions.current')}</Badge> : null}
        </div>
      </TableCell>
      <TableCell>
        <DeviceText session={session} />
      </TableCell>
      <TableCell className="font-mono text-xs">{session.ip ?? '—'}</TableCell>
      <TableCell>{formatDateTime(session.last_used_at, i18n.language)}</TableCell>
      <TableCell className="text-right">
        <Button
          size="sm"
          variant="outline"
          aria-label={t('me.sessions.revokeNamed', { name: label })}
          onClick={() => {
            onRevoke(session)
          }}
        >
          {t('me.sessions.revoke')}
        </Button>
      </TableCell>
    </TableRow>
  )
}

type Target = MySession | 'others'

/** Client and console sessions of the user; revoke one, or all but this one. */
export function SessionsTab() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const query = useListMySessions(PAGE)
  // `target` outlives `confirming`: the dialog keeps its texts while it animates closed.
  const [target, setTarget] = useState<Target | undefined>(undefined)
  const [confirming, setConfirming] = useState(false)
  const ask = (next: Target) => {
    setTarget(next)
    setConfirming(true)
  }
  const refresh = () => queryClient.invalidateQueries({ queryKey: getListMySessionsQueryKey() })

  const revoke = async () => {
    if (target === undefined) return
    if (target === 'others') {
      await revokeMyOtherSessions()
      await refresh()
      return
    }
    await revokeMySession(target.id)
    if (!target.current) {
      await refresh()
      return
    }
    // The server cleared the cookies: this browser is signed out.
    await navigate({ to: LOGIN_PATH, replace: true })
    queryClient.clear()
  }

  const sessions = query.data?.items ?? []
  const total = query.data?.total ?? 0
  const revokingCurrent = target !== 'others' && target?.current === true
  return (
    <Section title={t('me.sessions.title')} description={t('me.sessions.description')}>
      {query.isError ? (
        <ErrorAlert
          error={query.error}
          title={t('me.sessions.loadFailed')}
          onRetry={() => void query.refetch()}
        />
      ) : null}
      <div className="flex justify-end">
        <Button
          size="sm"
          variant="outline"
          disabled={sessions.filter((s) => !s.current).length === 0}
          onClick={() => {
            ask('others')
          }}
        >
          {t('me.sessions.revokeOthers')}
        </Button>
      </div>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t('me.sessions.columns.kind')}</TableHead>
            <TableHead>{t('me.sessions.columns.device')}</TableHead>
            <TableHead>{t('me.sessions.columns.ip')}</TableHead>
            <TableHead>{t('me.sessions.columns.lastUsed')}</TableHead>
            <TableHead className="w-px" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {sessions.map((session) => (
            <SessionRow key={session.id} session={session} onRevoke={ask} />
          ))}
        </TableBody>
      </Table>
      {total > sessions.length ? (
        <p className="text-xs text-muted-foreground">
          {t('me.sessions.truncated', { shown: sessions.length, total })}
        </p>
      ) : null}
      <p className="text-xs text-muted-foreground">{t('me.sessions.selfReported')}</p>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={
          target === 'others' ? t('me.sessions.revokeOthersTitle') : t('me.sessions.revokeTitle')
        }
        description={
          target === 'others'
            ? t('me.sessions.revokeOthersDescription')
            : revokingCurrent
              ? t('me.sessions.revokeCurrentDescription')
              : t('me.sessions.revokeDescription')
        }
        confirmLabel={target === 'others' ? t('me.sessions.revokeOthers') : t('me.sessions.revoke')}
        onConfirm={revoke}
      />
    </Section>
  )
}
