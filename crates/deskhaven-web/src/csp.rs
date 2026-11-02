//! Content-Security-Policy for the console (06-security §6).

/// Set it as `HttpConfig::content_security_policy`
/// (`HeaderValue::from_static(CSP)`).
///
/// Derived from what the built SPA needs, nothing more:
/// - `default-src 'self'`: everything else is same-origin only.
/// - `script-src 'self'`: no inline script (the theme bootstrap is the external
///   `theme-init.js`), no `eval`.
/// - `style-src 'self'`: the build emits one external stylesheet; the app only
///   sets CSS variables through the CSSOM, which CSP allows. A style *element*
///   or `style=""` attribute would be blocked.
/// - `font-src 'self'`: the Geist woff2 files are served from `/assets/`.
/// - `img-src 'self'`: only `/favicon.svg`; no `data:` URL anywhere in the build.
/// - `connect-src 'self'`: the SPA talks to `/admin/api` only.
/// - `object-src 'none'`, `base-uri 'none'`: no plugins, no `<base>` hijack.
/// - `form-action 'self'`: forms can only post back to this origin.
/// - `frame-ancestors 'none'`: cannot be framed (clickjacking).
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
font-src 'self'; img-src 'self'; connect-src 'self'; object-src 'none'; \
base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
