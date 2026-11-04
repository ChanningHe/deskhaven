// Rewrites the generated API client so path parameters are URL-encoded (see
// lib/encode-path-params.ts). Part of `pnpm gen:api`, between orval and eslint.
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'

import { encodePathParams } from './lib/encode-path-params.ts'

const GENERATED_DIR = new URL('../src/api/generated/', import.meta.url)

const files = readdirSync(GENERATED_DIR, { recursive: true, encoding: 'utf8' }).filter((file) =>
  file.endsWith('.ts'),
)
for (const file of files) {
  const url = new URL(file, GENERATED_DIR)
  const source = readFileSync(url, 'utf8')
  const encoded = encodePathParams(source)
  if (encoded !== source) writeFileSync(url, encoded)
}
