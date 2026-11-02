// Runs before first paint to apply the persisted theme and avoid a flash of the
// wrong color scheme. Mirrors src/lib/theme/theme.ts (THEME_STORAGE_KEY,
// resolveTheme); theme-init.test.ts keeps the two in sync.
;(function () {
  var theme = 'system'
  try {
    var stored = window.localStorage.getItem('deskhaven.theme')
    if (stored === 'light' || stored === 'dark' || stored === 'system') theme = stored
  } catch {
    // Storage unavailable: fall back to the system preference.
  }
  var prefersDark =
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-color-scheme: dark)').matches
  var isDark = theme === 'dark' || (theme === 'system' && prefersDark)
  document.documentElement.classList.toggle('dark', isDark)
})()
