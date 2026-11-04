import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useSetAddressBookShare } from '@/api/generated/address-books/address-books'
import { useListGroups } from '@/api/generated/groups/groups'
import type { SubjectKind } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { NativeSelect } from '@/components/common/native-select'
import { UserCombobox } from '@/components/common/user-combobox'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'

import { invalidateAddressBooks } from '../invalidate'
import { DEFAULT_RULE, RULES } from '../rules'

/** Most groups offered in the select (the API's page size limit). */
const MAX_GROUPS = 200

/**
 * "Grant to": a user (searched) or a group (chosen), with a rule. Picking a user grants at
 * once; a group needs the button.
 */
export function AddShare({ guid }: { guid: string }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [kind, setKind] = useState<SubjectKind>('user')
  const [rule, setRule] = useState<number>(DEFAULT_RULE)
  const [group, setGroup] = useState('')
  const groups = useListGroups(
    { page_size: MAX_GROUPS, sort: 'name' },
    {
      query: { enabled: kind === 'group' },
    },
  )
  const mutation = useSetAddressBookShare()

  const grant = async (subjectId: number): Promise<boolean> => {
    const done = await mutation
      .mutateAsync({ guid, data: { subject_type: kind, subject_id: subjectId, rule } })
      .catch(() => undefined)
    if (done === undefined) return false
    await invalidateAddressBooks(queryClient).catch(() => undefined)
    return true
  }

  return (
    <div className="grid gap-2 rounded-lg border p-3">
      <div className="flex flex-wrap items-end gap-2">
        <div className="grid gap-1">
          <Label htmlFor={`${id}-kind`}>{t('addressBooks.shares.grantTo')}</Label>
          <NativeSelect
            id={`${id}-kind`}
            value={kind}
            onChange={(e) => {
              setKind(e.target.value === 'group' ? 'group' : 'user')
            }}
          >
            <option value="user">{t('addressBooks.shares.kinds.user')}</option>
            <option value="group">{t('addressBooks.shares.kinds.group')}</option>
          </NativeSelect>
        </div>
        <div className="grid gap-1">
          <Label htmlFor={`${id}-rule`}>{t('addressBooks.shares.rule')}</Label>
          <NativeSelect
            id={`${id}-rule`}
            value={rule}
            onChange={(e) => {
              setRule(Number(e.target.value))
            }}
          >
            {RULES.map((value) => (
              <option key={value} value={value}>
                {t(`addressBooks.shares.rules.${value}`)}
              </option>
            ))}
          </NativeSelect>
        </div>
      </div>
      {kind === 'user' ? (
        <UserCombobox
          label={t('addressBooks.shares.userSearch')}
          listLabel={t('addressBooks.shares.userCandidates')}
          busy={mutation.isPending}
          foundText={(count) => t('addressBooks.shares.userFound', { count })}
          noMatchText={t('addressBooks.shares.userNoMatch')}
          onPick={(user) => grant(user.id)}
        />
      ) : (
        <div className="flex flex-wrap items-end gap-2">
          <div className="grid gap-1">
            <Label htmlFor={`${id}-group`}>{t('addressBooks.shares.group')}</Label>
            <NativeSelect
              id={`${id}-group`}
              value={group}
              onChange={(e) => {
                setGroup(e.target.value)
              }}
            >
              <option value="">{t('addressBooks.shares.groupPlaceholder')}</option>
              {groups.data?.items.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.name}
                </option>
              ))}
            </NativeSelect>
          </div>
          <Button
            size="sm"
            disabled={group === '' || mutation.isPending}
            onClick={() => {
              void grant(Number(group)).then((done) => {
                if (done) setGroup('')
              })
            }}
          >
            {t('addressBooks.shares.grant')}
          </Button>
        </div>
      )}
      {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
    </div>
  )
}
