# Test certificates

These certificates are only used by the unit tests in `src/controller.rs` to
verify that the client trusts a private CA passed via `--cacert`.

- `ca.pem` — a self-signed CA certificate.
- `server.pem`, `server-key.pem` — a server certificate for `127.0.0.1` signed
  by that CA, together with its private key.

They are valid for 100 years and are not used anywhere else. Do not use them
for anything other than tests.
