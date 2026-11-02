import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Brand } from '@/features/shell/components/brand'

// Static placeholder: sign-in is wired up once the admin API exists.
function preventSubmit(event: SubmitEvent<HTMLFormElement>) {
  // Never let the browser submit (a GET would leak the password into the URL).
  event.preventDefault()
}

export function LoginPage() {
  const { t } = useTranslation()
  return (
    <main className="flex min-h-svh items-center justify-center bg-muted/40 px-4 py-12">
      <div className="w-full max-w-sm space-y-6 rounded-xl border bg-card p-6 shadow-sm">
        <Brand />
        <h1 className="text-lg font-semibold tracking-tight">{t('login.title')}</h1>
        <form method="post" className="space-y-4" onSubmit={preventSubmit}>
          <div className="space-y-2">
            <Label htmlFor="login-username">{t('login.username')}</Label>
            <Input id="login-username" name="username" autoComplete="username" required />
          </div>
          <div className="space-y-2">
            <Label htmlFor="login-password">{t('login.password')}</Label>
            <Input
              id="login-password"
              name="password"
              type="password"
              autoComplete="current-password"
              required
            />
          </div>
          <Button type="submit" className="w-full" disabled>
            {t('login.submit')}
          </Button>
        </form>
        <p className="text-xs text-muted-foreground">{t('login.notice')}</p>
      </div>
    </main>
  )
}
