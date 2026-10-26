Public test-only self-signed localhost identities. PFX password: fixture.
Generated with .NET CertificateRequest using ephemeral RSA keys, SHA-256,
localhost DNS SAN, serverAuth EKU, valid 2026-01-01 through 2036-01-01.
The two certificates intentionally differ to test control/data identity binding.
No production credentials. Source releases may include only these exact public
fixtures, pinned by path and SHA-256 in scripts/release-source-inventory.json.
Never include them in application/installer payloads, replace them with production
identities, or install their certificates in an operating-system trust store.
Tests trust them only in private connectors. The fixture password is public.
