# OIDC test fixtures — throwaway, not secrets

`idp-key-a.pem` and `idp-key-b.pem` are 2048-bit RSA keys (PKCS#1) generated only for the
mock identity provider of the OIDC tests (`../../common/oidc_idp.rs`, also used by the
contract tests). They sign test ID tokens and protect nothing. They are never used by
deskhaven itself, which holds no RSA private key (it only verifies IdP signatures with
public JWKS keys).

Secret scanners may flag them: this is expected. Regenerate with
`openssl genrsa -traditional -out idp-key-a.pem 2048` if ever needed.
