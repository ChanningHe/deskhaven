// Command line examples for an installed RustDesk client (core_main.rs of the pinned
// client: `--config`, `--deploy`, `--assign`). The token is a placeholder: API tokens
// are created on their own page and shown once.

/** What to replace with a real API token (`dhk_` is the token prefix). */
export const TOKEN_PLACEHOLDER = 'dhk_YOUR_API_TOKEN'

/** Applies ID server, relay, API server and key from the configuration string. */
export function configCommand(configString: string): string {
  return `rustdesk --config ${configString}`
}

/** Registers the device with the server (needs `--deploy` rights on the token). */
export function deployCommand(): string {
  return `rustdesk --deploy --token ${TOKEN_PLACEHOLDER}`
}

/** Assigns the device to a user and a device group. */
export function assignCommand(): string {
  return `rustdesk --assign --token ${TOKEN_PLACEHOLDER} --user_name alice --device_group_name Office`
}
