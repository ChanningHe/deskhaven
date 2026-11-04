import { useQueryClient } from '@tanstack/react-query'
import { TriangleAlert } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  removeAddressBookShare,
  useListAddressBookShares,
  useSetAddressBookShare,
} from '@/api/generated/address-books/address-books'
import type { ShareView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { NativeSelect } from '@/components/common/native-select'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'

import { invalidateAddressBooks } from '../invalidate'
import { RULES } from '../rules'
import { AddShare } from './add-share'

/**
 * Who a book is granted to. A rule 1 grant already hands the sharee's client every entry's
 * password (it connects with them), so the warning is always on screen (06 §4.2).
 */
export function BookShares({ guid }: { guid: string }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const shares = useListAddressBookShares(guid)
  const change = useSetAddressBookShare()
  const [revoking, setRevoking] = useState<ShareView | null>(null)
  const [confirming, setConfirming] = useState(false)

  const setRule = async (share: ShareView, rule: number) => {
    const done = await change
      .mutateAsync({
        guid,
        data: { subject_type: share.subject_type, subject_id: share.subject_id, rule },
      })
      .catch(() => undefined)
    if (done !== undefined) await invalidateAddressBooks(queryClient)
  }

  return (
    <section className="grid gap-3">
      <p
        role="note"
        className="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-sm"
      >
        <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0 text-warning" />
        {t('addressBooks.shares.passwordWarning')}
      </p>
      <AddShare guid={guid} />
      {shares.error ? (
        <ErrorAlert error={shares.error} onRetry={() => void shares.refetch()} />
      ) : null}
      {change.error ? <ErrorAlert error={change.error} /> : null}
      {shares.data?.items.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t('addressBooks.shares.empty')}</p>
      ) : null}
      <ul className="grid gap-2 text-sm">
        {shares.data?.items.map((share) => {
          const name = share.subject_name ?? t('addressBooks.shares.unknownSubject')
          return (
            <li
              key={`${share.subject_type}:${share.subject_id}`}
              className="flex flex-wrap items-center justify-between gap-2"
            >
              <span className="flex items-center gap-2">
                <Badge variant="outline">
                  {t(`addressBooks.shares.kinds.${share.subject_type}`)}
                </Badge>
                {name}
              </span>
              <span className="flex items-center gap-1">
                <NativeSelect
                  aria-label={t('addressBooks.shares.ruleOf', { name })}
                  value={share.rule}
                  disabled={change.isPending}
                  onChange={(e) => void setRule(share, Number(e.target.value))}
                >
                  {RULES.map((rule) => (
                    <option key={rule} value={rule}>
                      {t(`addressBooks.shares.rules.${rule}`)}
                    </option>
                  ))}
                </NativeSelect>
                <Button
                  size="xs"
                  variant="ghost"
                  aria-label={t('addressBooks.shares.revoke', { name })}
                  onClick={() => {
                    setRevoking(share)
                    setConfirming(true)
                  }}
                >
                  {t('addressBooks.shares.revokeShort')}
                </Button>
              </span>
            </li>
          )
        })}
      </ul>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('addressBooks.shares.confirmTitle')}
        description={t('addressBooks.shares.confirmDescription', {
          name: revoking?.subject_name ?? t('addressBooks.shares.unknownSubject'),
        })}
        confirmLabel={t('addressBooks.shares.revokeShort')}
        onConfirm={async () => {
          if (revoking === null) return
          await removeAddressBookShare(guid, revoking.subject_type, revoking.subject_id)
          await invalidateAddressBooks(queryClient)
        }}
      />
    </section>
  )
}
