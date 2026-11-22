import { describe, expect, it } from 'vitest'

import { groupByCategory, unsavedCount } from './categories'
import { editSetting, NO_DRAFTS, stageReset } from './drafts'
import { sampleSettings } from './test-data'

const SETTINGS = sampleSettings()
function find(key: string) {
  const found = SETTINGS.find((s) => s.key === key)
  if (found === undefined) throw new Error(`no ${key}`)
  return found
}
function category(name: string) {
  const found = groupByCategory(SETTINGS).find((c) => c.name === name)
  if (found === undefined) throw new Error(`no category ${name}`)
  return found
}

describe('unsaved count of a category', () => {
  it('is zero without drafts', () => {
    expect(unsavedCount(category('auth'), NO_DRAFTS)).toBe(0)
  })

  it('counts edits and staged resets of its own settings only', () => {
    let drafts = editSetting(NO_DRAFTS, find('auth.session_ttl'), '60d')
    drafts = stageReset(drafts, find('auth.registration'))
    drafts = editSetting(drafts, find('smtp.host'), 'mail.example.com')

    expect(unsavedCount(category('auth'), drafts)).toBe(2)
    expect(unsavedCount(category('smtp'), drafts)).toBe(1)
    expect(unsavedCount(category('devices'), drafts)).toBe(0)
  })
})
