import { useTranslation } from 'react-i18next'

import {
  useListLdapDirectories,
  useListOidcProviderConfigs,
} from '@/api/generated/identity/identity'
import { useListSettings } from '@/api/generated/settings/settings'

import type { IdentityTab } from './identity-search'
import { shownDirectory } from './ldap-form'
import { isTfaChoice, TFA_KEY } from './tfa-policy'

export type TabSummaries = Record<IdentityTab, string | undefined>

/** Undefined while loading, after a failure, and for the 2FA policy unless the viewer is the owner. */
export function useTabSummaries(isOwner: boolean): TabSummaries {
  const { t } = useTranslation()
  const providers = useListOidcProviderConfigs()
  const directories = useListLdapDirectories()
  const settings = useListSettings({ query: { enabled: isOwner } })

  const enabledProviders = providers.data?.items.filter((p) => p.enabled).length
  const directory = directories.data === undefined ? undefined : shownDirectory(directories.data)
  const policy = settings.error ? undefined : settings.data?.find((s) => s.key === TFA_KEY)

  return {
    oidc:
      providers.error || enabledProviders === undefined
        ? undefined
        : enabledProviders === 0
          ? t('identity.tabs.oidcNone')
          : t('identity.tabs.oidcEnabled', { count: enabledProviders }),
    ldap:
      directories.error || directories.data === undefined
        ? undefined
        : directory === undefined
          ? t('identity.ldap.status.unconfigured')
          : directory.enabled
            ? t('identity.ldap.status.enabled')
            : t('identity.ldap.status.disabled'),
    tfa:
      !isOwner || policy === undefined
        ? undefined
        : isTfaChoice(policy.value) && policy.value !== 'off'
          ? t(`identity.tfa.choices.${policy.value}`)
          : t('identity.tabs.tfaOff'),
  }
}
