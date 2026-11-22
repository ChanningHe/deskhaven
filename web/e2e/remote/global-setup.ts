// Starts the remote stack once per run and stops it in the returned teardown.
import { fileURLToPath } from 'node:url'

import { BASE_URL_ENV, STACK_ENV } from './support/env.ts'
import { startStack } from './support/stack.ts'

export const RESULTS = fileURLToPath(new URL('../../e2e-results/remote', import.meta.url))

export default async function globalSetup(): Promise<() => Promise<void>> {
  const stack = await startStack(RESULTS)
  process.env[STACK_ENV] = JSON.stringify(stack.info)
  process.env[BASE_URL_ENV] = stack.info.baseUrl
  process.stdout.write(`remote stack: ${JSON.stringify(stack.info, null, 1)}\n`)
  return stack.stop
}
