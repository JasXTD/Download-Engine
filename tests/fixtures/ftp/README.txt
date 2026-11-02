Public test-only self-signed localhost identities. PFX password: fixture.
Generated with .NET CertificateRequest using ephemeral RSA keys, SHA-256,
localhost DNS SAN, serverAuth EKU, valid 2026-01-01 through 2036-01-01.
The two certificates intentionally differ to test control/data identity binding.
The alternate certificate uses a different issuer name (with the same localhost
DNS SAN) so OpenSSL can distinguish both trust anchors before the pinning check.
No production credentials. Source releases may include only these exact public
fixtures, pinned by path and SHA-256 in scripts/release-source-inventory.json.
Never include them in application/installer payloads, replace them with production
identities, or install their certificates in an operating-system trust store.
Tests trust them only in private connectors. The fixture password is public.

macOS tests instead generate two short-lived (30-day) localhost identities with
the installed OpenSSL command through scripts/Create-TestTlsFixtures.sh. This
respects macOS certificate-validity rules without weakening TLS verification.
The certificates and identities are held in memory and their temporary files
are removed immediately. Nothing is added to an OS trust store or release bundle.
