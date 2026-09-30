# Untrusted loopback TLS identity (test only)

`untrusted-loopback.cert.pem` and `untrusted-loopback.key.pem` are a throwaway
self-signed RSA-2048 certificate and its unencrypted private key for
`CN=vsift-untrusted-loopback-test`, `subjectAltName=IP:127.0.0.1`. They protect
nothing and are published on purpose.

`tests/p13_install_transaction.rs` serves them from a local TLS server so that a
managed download meets a certificate no trust store accepts and must fail with
the typed `tls` reason (D-07). No client ever trusts this certificate; nothing
outside that test reads these files, and no build packages them.

Generated on 2026-09-30 with OpenSSL 1.1.1k:

```console
openssl req -x509 -newkey rsa:2048 -nodes -sha256 \
  -keyout untrusted-loopback.key.pem -out untrusted-loopback.cert.pem \
  -days 36500 -subj "/CN=vsift-untrusted-loopback-test" \
  -addext "subjectAltName=IP:127.0.0.1" -addext "extendedKeyUsage=serverAuth"
```
