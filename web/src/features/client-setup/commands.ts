// The token is a placeholder: API tokens are created on their own page and shown once.

export const TOKEN_PLACEHOLDER = 'dhk_YOUR_API_TOKEN'

export function configCommand(configString: string): string {
  return `rustdesk --config ${configString}`
}

/** Needs `--deploy` rights on the token. */
export function deployCommand(): string {
  return `rustdesk --deploy --token ${TOKEN_PLACEHOLDER}`
}

export function assignCommand(): string {
  return `rustdesk --assign --token ${TOKEN_PLACEHOLDER} --user_name alice --device_group_name Office`
}
