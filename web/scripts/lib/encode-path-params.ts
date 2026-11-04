// Orval interpolates path parameters into the request URL as they are. A name or id with
// `/`, `?`, `#` or `..` (entry ids and tag names are chosen by sharees, not only by
// administrators) would then address another resource. This rewrites the generated URL
// templates so every path parameter goes through `encodeURIComponent`.
// Run by `pnpm gen:api` after orval; unit tested in encode-path-params.test.ts.

/** Template literals of generated URLs: they start at the admin API root. */
const URL_TEMPLATE = /`(\/admin\/api\/v1[^`]*)`/g
const PATH_PARAM = /\$\{(\w+)\}/g

/** Encodes the `${param}` placeholders in the path part (before any `?`) of one template. */
function encodeTemplate(template: string): string {
  const query = template.indexOf('?')
  const path = query === -1 ? template : template.slice(0, query)
  const rest = query === -1 ? '' : template.slice(query)
  return path.replace(PATH_PARAM, (_, name: string) => `\${encodeURIComponent(${name})}`) + rest
}

/** `source` with every generated URL template's path parameters encoded. */
export function encodePathParams(source: string): string {
  return source.replace(URL_TEMPLATE, (_, template: string) => `\`${encodeTemplate(template)}\``)
}
