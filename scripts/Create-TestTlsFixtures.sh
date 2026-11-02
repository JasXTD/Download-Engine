#!/usr/bin/env bash
# Test-only localhost identities. Never install these in a system trust store.
set -euo pipefail
umask 077
output=${1:?Usage: Create-TestTlsFixtures.sh EMPTY_OUTPUT_DIRECTORY}
command -v openssl >/dev/null || { echo 'Tests need the installed OpenSSL command.' >&2; exit 1; }
mkdir -p "$output"
output=$(cd "$output" && pwd)
for name in server alternate; do
  [[ ! -e "$output/$name.cer" && ! -e "$output/$name.pfx" ]] || { echo 'Fixture destination is not empty' >&2; exit 1; }
done
work=$(mktemp -d "${TMPDIR:-/tmp}/de-test-tls.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
for name in server alternate; do
  cat > "$work/request.cnf" <<CONFIG
[req]
distinguished_name=subject
x509_extensions=extensions
prompt=no
[subject]
CN=Download Engine test $name ${work##*.}
[extensions]
subjectAltName=DNS:localhost
basicConstraints=critical,CA:TRUE
keyUsage=critical,digitalSignature,keyEncipherment,keyCertSign
extendedKeyUsage=serverAuth
subjectKeyIdentifier=hash
CONFIG
  openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 30 \
    -config "$work/request.cnf" -keyout "$work/key.pem" -out "$work/cert.pem" 2> "$work/generation.log" || { cat "$work/generation.log" >&2; exit 1; }
  openssl x509 -in "$work/cert.pem" -outform DER -out "$output/$name.cer"
  # Legacy PKCS#12 encoding for the platform import APIs; the password and key
  # are synthetic test data, not protection for a production identity.
  openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" \
    -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 \
    -passout pass:fixture -out "$output/$name.pfx"
done
