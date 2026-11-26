import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useTestLdapConnection, useTestLdapUser } from '@/api/generated/identity/identity'
import type { LdapDiagnosticView, NewLdapDirectoryBody } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { SectionCard } from '@/components/common/section-card'
import { Button } from '@/components/ui/button'
import { TextField } from '@/features/auth/components/text-field'
import { SECRET_MUTATION } from '@/features/users/secret-mutation'

import { DiagnosticResult } from './diagnostic-result'
import { FieldProblems } from './form-controls'

export type TestTarget = { directory_id: number } | { directory: NewLdapDirectoryBody }

/** Kept outside the directory form: its buttons must never submit the directory. */
export function LdapDiagnostics({ target }: { target: TestTarget }) {
  const { t } = useTranslation()
  const id = useId()
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const connection = useTestLdapConnection(SECRET_MUTATION)
  const user = useTestLdapUser(SECRET_MUTATION)
  // The newest of the two answers is the one shown.
  const [result, setResult] = useState<LdapDiagnosticView | undefined>(undefined)
  const [error, setError] = useState<unknown>(undefined)
  const pending = connection.isPending || user.isPending

  const run = async (test: () => Promise<LdapDiagnosticView>) => {
    setError(undefined)
    setResult(undefined)
    try {
      setResult(await test())
    } catch (caught) {
      setError(caught)
    }
  }
  const testConnection = () => run(() => connection.mutateAsync({ data: target }))
  const testUser = () =>
    run(() =>
      user.mutateAsync({
        data: { ...target, username: username.trim(), password: password === '' ? null : password },
      }),
    )

  return (
    <SectionCard
      title={t('identity.diagnostics.title')}
      description={
        'directory_id' in target
          ? t('identity.diagnostics.savedHint')
          : t('identity.diagnostics.unsavedHint')
      }
    >
      <div className="grid max-w-xl gap-6">
        <div>
          <Button variant="outline" disabled={pending} onClick={() => void testConnection()}>
            {t('identity.diagnostics.testConnection')}
          </Button>
        </div>
        <div className="grid gap-4">
          <div className="grid gap-2">
            <div className="grid gap-4 sm:grid-cols-2">
              <TextField
                id={`${id}-username`}
                label={t('identity.diagnostics.username')}
                value={username}
                autoComplete="off"
                onChange={(event) => {
                  setUsername(event.target.value)
                }}
              />
              <TextField
                id={`${id}-password`}
                label={t('identity.diagnostics.password')}
                type="password"
                autoComplete="new-password"
                value={password}
                onChange={(event) => {
                  setPassword(event.target.value)
                }}
              />
            </div>
            <p className="text-xs text-muted-foreground">
              {t('identity.diagnostics.passwordHint')}
            </p>
          </div>
          <div>
            <Button
              variant="outline"
              disabled={pending || username.trim() === ''}
              onClick={() => void testUser()}
            >
              {t('identity.diagnostics.testUser')}
            </Button>
          </div>
        </div>
        {error === undefined ? null : (
          <div className="grid gap-1">
            <ErrorAlert error={error} />
            <FieldProblems error={error} />
          </div>
        )}
        {result === undefined ? null : <DiagnosticResult result={result} />}
      </div>
    </SectionCard>
  )
}
