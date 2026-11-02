import { useNavigate } from '@tanstack/react-router'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { LANGUAGE_NAMES, setLanguage, SUPPORTED_LANGUAGES } from '@/i18n'
import { THEMES } from '@/lib/theme/theme'
import { useTheme } from '@/lib/theme/use-theme'
import { useMe } from '@/features/auth/session'

import { ME_NAV_ITEM, navSectionsFor } from './nav-items'

export interface CommandItem {
  /** Unique id; also what Base UI uses as the item's identity. */
  value: string
  /** Searchable, visible label. */
  label: string
  run: () => void
}

export interface CommandGroup {
  value: string
  items: CommandItem[]
}

/**
 * Everything the command palette (08 §3.1) offers: the pages the user may
 * open, then preferences. Device / user search and common actions join here
 * once their APIs exist (M3-04, M3-05).
 */
export function useCommandGroups(): CommandGroup[] {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { setTheme } = useTheme()
  const { role } = useMe()

  return useMemo(() => {
    const visible = navSectionsFor(role).flat()
    const navItems = visible.some((item) => item.key === ME_NAV_ITEM.key)
      ? visible
      : [...visible, ME_NAV_ITEM]

    const pages: CommandItem[] = navItems.map((item) => ({
      value: `page:${item.key}`,
      label: t(`nav.${item.key}`),
      run: () => void navigate({ to: item.to }),
    }))
    const themes: CommandItem[] = THEMES.map((theme) => ({
      value: `theme:${theme}`,
      label: `${t('theme.label')}: ${t(`theme.${theme}`)}`,
      run: () => {
        setTheme(theme)
      },
    }))
    const languages: CommandItem[] = SUPPORTED_LANGUAGES.map((language) => ({
      value: `language:${language}`,
      label: `${t('language.label')}: ${LANGUAGE_NAMES[language]}`,
      run: () => void setLanguage(language),
    }))

    return [
      { value: t('shell.commandGroupPages'), items: pages },
      { value: t('shell.commandGroupPreferences'), items: [...themes, ...languages] },
    ]
  }, [t, navigate, setTheme, role])
}
