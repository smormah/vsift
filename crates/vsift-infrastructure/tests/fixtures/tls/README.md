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

# Link test root and server identity (test only, ADR 0028)

`link-test-ca.cert.pem` is a throwaway self-signed RSA-2048 root,
`CN=vsift-link-test-ca`; `link-test-server.cert.pem` and
`link-test-server.key.pem` are a server certificate it signed and its
unencrypted private key, for `CN=video.example.test` with
`subjectAltName=DNS:video.example.test,DNS:cdn.example.test,DNS:private.example.test`.
They protect nothing and are published on purpose; the root's private key was
not kept.

`tests/link_transfer.rs` serves the server identity from a local HTTPS server on
`127.0.0.1` and trusts the root only through the development-only link test
route (`install-test-hooks`), so `ingest --url`'s fetch meets a real TLS server
without the network. No release build can trust this root, and no build packages
these files.

Generated on 2026-10-21 with OpenSSL 1.1.1k:

```console
openssl req -x509 -newkey rsa:2048 -nodes -sha256 -keyout link-test-ca.key.pem \
  -out link-test-ca.cert.pem -days 36500 -subj "/CN=vsift-link-test-ca" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"
openssl req -newkey rsa:2048 -nodes -sha256 -keyout link-test-server.key.pem \
  -out server.csr -subj "/CN=video.example.test"
openssl x509 -req -in server.csr -CA link-test-ca.cert.pem -CAkey link-test-ca.key.pem \
  -CAcreateserial -out link-test-server.cert.pem -days 36500 -sha256 -extfile leaf.ext
```

where `leaf.ext` holds `basicConstraints=critical,CA:FALSE`,
`keyUsage=critical,digitalSignature,keyEncipherment`, `extendedKeyUsage=serverAuth`
and the `subjectAltName` above.
