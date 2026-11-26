// Orval interpolates path parameters as they are. A `/`, `?`, `#` or `..` in a name chosen by a
// sharee would then address another resource, so every one goes through `encodeURIComponent`.

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
