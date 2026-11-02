//! Placeholder served when the binary was compiled without `web/dist`.
//! No inline script or style: it must work under the strict CSP.

pub(crate) const NOT_BUILT_HTML: &str = "<!doctype html>
<html lang=\"en\">
  <head>
    <meta charset=\"UTF-8\" />
    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />
    <title>deskhaven: web console not built</title>
  </head>
  <body>
    <h1>The web console is not built</h1>
    <p>This binary was compiled without the web console assets.</p>
    <p>Rebuild with <code>cargo run -p xtask -- dist</code>.</p>
  </body>
</html>
";
