import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useTestLdapConnection, useTestLdapUser } from '@/api/generated/identity/identity'
import type { LdapDiagnosticView, NewLdapDirectoryBody } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { TextField } from '@/features/auth/components/text-field'
import { SECRET_MUTATION } from '@/features/users/secret-mutation'

import { DiagnosticResult } from './diagnostic-result'
import { FieldProblems } from './form-controls'

/** What to test: the saved directory, or the form as typed. */
export type TestTarget = { directory_id: number } | { directory: NewLdapDirectoryBody }

/** "Test connection" and "test user" with their step-by-step result. Owner only. */
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
    <div className="grid gap-3 rounded-lg border p-4">
      <h3 className="text-sm font-semibold">{t('identity.diagnostics.title')}</h3>
      <p className="text-xs text-muted-foreground">
        {'directory_id' in target
          ? t('identity.diagnostics.savedHint')
          : t('identity.diagnostics.unsavedHint')}
      </p>
      <div>
        <Button variant="outline" disabled={pending} onClick={() => void testConnection()}>
          {t('identity.diagnostics.testConnection')}
        </Button>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
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
      <p className="text-xs text-muted-foreground">{t('identity.diagnostics.passwordHint')}</p>
      <div>
        <Button
          variant="outline"
          disabled={pending || username.trim() === ''}
          onClick={() => void testUser()}
        >
          {t('identity.diagnostics.testUser')}
        </Button>
      </div>
      {error === undefined ? null : (
        <div className="grid gap-1">
          <ErrorAlert error={error} />
          <FieldProblems error={error} />
        </div>
      )}
      {result === undefined ? null : <DiagnosticResult result={result} />}
    </div>
  )
}
