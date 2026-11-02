// Leaving the SPA for another site (an OIDC identity provider). A module of its
// own so tests can replace it: jsdom does not navigate.

export function assignLocation(url: string): void {
  window.location.assign(url)
}
