import { useTranslation } from 'react-i18next'

import { formatDateTime } from '@/lib/format'

import { diffSnapshots, purgeSummary, showValue } from '../snapshot'
import type { SnapshotChange } from '../snapshot'

interface SnapshotViewProps {
  action: string
  before: unknown
  after: unknown
}

/**
 * What a console audit row changed, read-only. A retention purge gets its one-line summary;
 * object snapshots a changed-members list; anything else the stored JSON whole. Values are
 * text nodes only: they may carry text a device or a user typed.
 */
export function SnapshotView({ action, before, after }: SnapshotViewProps) {
  const { t, i18n } = useTranslation()
  const purge = purgeSummary(action, after)
  if (purge !== undefined) {
    return (
      <p className="text-sm">
        {t('audit.snapshot.purge', {
          deleted: purge.deleted,
          kind: purge.kind,
          cutoff: formatDateTime(purge.cutoff, i18n.language),
          batch: purge.batch,
        })}
      </p>
    )
  }
  const diff = diffSnapshots(before, after)
  if (diff === undefined) {
    return (
      <div className="grid gap-3">
        <Json title={t('audit.snapshot.before')} value={before} />
        <Json title={t('audit.snapshot.after')} value={after} />
      </div>
    )
  }
  if (diff.changes.length === 0) {
    return <p className="text-sm text-muted-foreground">{t('audit.snapshot.noChange')}</p>
  }
  return (
    <div className="grid gap-2">
      <ul aria-label={t('audit.snapshot.changes')} className="grid gap-2 text-sm">
        {diff.changes.map((change) => (
          <ChangeRow key={change.key} change={change} />
        ))}
      </ul>
      {diff.unchanged === 0 ? null : (
        <p className="text-xs text-muted-foreground">
          {t('audit.snapshot.unchanged', { count: diff.unchanged })}
        </p>
      )}
    </div>
  )
}

function ChangeRow({ change }: { change: SnapshotChange }) {
  const { t } = useTranslation()
  return (
    <li className="grid gap-1 rounded-lg border p-2">
      <span className="font-mono text-xs font-medium">
        <bdi>{change.key}</bdi>
      </span>
      {change.kind === 'added' ? null : (
        <Value label={t('audit.snapshot.before')} value={change.before} />
      )}
      {change.kind === 'removed' ? null : (
        <Value label={t('audit.snapshot.after')} value={change.after} />
      )}
    </li>
  )
}

function Value({ label, value }: { label: string; value: unknown }) {
  return (
    <div className="grid grid-cols-[4rem_1fr] gap-2">
      <span className="text-xs text-muted-foreground">{label}</span>
      <pre className="min-w-0 font-mono text-xs break-words whitespace-pre-wrap">
        <bdi>{showValue(value)}</bdi>
      </pre>
    </div>
  )
}

function Json({ title, value }: { title: string; value: unknown }) {
  return (
    <div className="grid gap-1">
      <h4 className="text-xs font-medium text-muted-foreground">{title}</h4>
      <pre className="rounded-lg border p-2 font-mono text-xs break-words whitespace-pre-wrap">
        <bdi>{showValue(value)}</bdi>
      </pre>
    </div>
  )
}
