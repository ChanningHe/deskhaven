/** `ABCDEFGH…` as `ABCD EFGH …`, easier to type into an authenticator. */
export function groupedSecret(secret: string): string {
  return secret.replace(/(.{4})(?=.)/g, '$1 ')
}
