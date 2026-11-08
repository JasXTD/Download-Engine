# Torrent interface binding lab

This lab uses generated ASCII data, private `.torrent` metadata and two Docker
**internal** networks. It never uses public peers, trackers, files or DNS.
`lab-peer.py` uses only Python's standard library; no pip packages are needed.
Use an already reviewed Debian/Rust image with Python 3, OpenSSL and the project's
approved Cargo sources. Do not download dependencies as part of a test run.

The development test used the existing local image
`sha256:b36c246742b4d323472588f789601be34eb4af053255e2af53969078ee8dcec7`.

## Preparation

Create `de-torrent-binding-other` (`172.30.240.0/24`) and
`de-torrent-binding-vpn` (`172.30.241.0/24`) with `docker network create --internal`.
Create these disposable containers from the reviewed local image:

- `de-torrent-binding-20261108`: connect the other network first as
  `172.30.240.2` (eth0), then the VPN network as `172.30.241.2` (eth1).
- `de-torrent-binding-peer-20261108`: VPN address `172.30.241.3` and other address
  `172.30.240.3`.

Drop all capabilities, enable `no-new-privileges`, and apply CPU, memory and PID
limits. The client lab initially used NET_ADMIN/NET_RAW for network diagnostics;
the binding tests must also pass under `docker exec --user 65534` without them.
Do not mount a real profile, download folder, Docker socket, home or credentials.
Mount only `torrent`, `vendor` and checksum-reviewed Cargo sources read-only as
`/input/torrent`, `/input/vendor` and `/input/registry` in the client container.

Copy `build-lab.sh` into that container and run it with `sh`. It snapshots source
files, excluding build caches, and compiles offline with the `lab` feature. Copy
the resulting `/work/target/debug/de-torrent` to `/work/de-torrent` in both
containers. Keep that lab-enabled binary out of release packages.

In the peer container, generate a temporary one-day self-signed certificate for
`fixture.test` using its installed OpenSSL:

```sh
mkdir -p /work
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -subj /CN=fixture.test -addext subjectAltName=DNS:fixture.test \
  -keyout /work/fixture-key.pem -out /work/fixture-cert.pem
```

Keep the private key inside that disposable container. Copy only the certificate
to `/work/fixture-cert.pem` in the client. Copy and start `lab-peer.py` in the peer;
it generates `/work/fixtures` and records requests in `/work/traffic.jsonl`.
Copy `/work/fixtures` to the client. No executable content is generated.

## Checks

Run the compiled engine test binary with `DE_BINDING_LAB_INTERFACE=eth1` and
`SSL_CERT_FILE=/work/fixture-cert.pem`, first normally, then the
`network_binding_tests` filter as UID 65534. The certificate override is local to
that test process; certificate and hostname checks remain enabled.

Then, from the repository root on the host:

```powershell
node tests/torrent-binding/run.cjs
```

This driver accepts only the fixed container/network names above. It verifies
hashes, actual encrypted TCP indicators, peer/HTTP tracker/UDP tracker/web-seed
transfers, stable paused files, Stop all, disappearance of the selected interface,
blocked restart, and recovery from verified saved progress after reconnection.
The engine tests separately exercise bound encrypted uTP, incoming/outgoing peer
indicators, HTTPS hostname rejection, DHT bootstrap, and UDP/TCP DNS. The traffic
audit permits only the explicit unbound reachability control on the other network.

Results go under `artifacts/torrent-binding-20261108`. Preserve reports, then remove
only these two test containers and networks, temporary binary copies and generated
data. Never stop unrelated containers or alter host network settings.
