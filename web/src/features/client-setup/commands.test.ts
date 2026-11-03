import { describe, expect, it } from 'vitest'

import { assignCommand, configCommand, deployCommand, TOKEN_PLACEHOLDER } from './commands'

describe('client command examples', () => {
  it('imports the configuration string with rustdesk --config', () => {
    expect(configCommand('abc=')).toBe('rustdesk --config abc=')
  })

  it('deploys with an API token placeholder carrying the dhk_ prefix', () => {
    expect(TOKEN_PLACEHOLDER.startsWith('dhk_')).toBe(true)
    expect(deployCommand()).toBe(`rustdesk --deploy --token ${TOKEN_PLACEHOLDER}`)
  })

  it('assigns to a user and a device group with the same placeholder', () => {
    expect(assignCommand()).toBe(
      `rustdesk --assign --token ${TOKEN_PLACEHOLDER} --user_name alice --device_group_name Office`,
    )
  })
})
