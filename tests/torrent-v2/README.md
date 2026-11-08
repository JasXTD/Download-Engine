# BitTorrent v2 and hybrid interoperability lab

This test generates harmless binary patterns and empty files. It never joins a
public swarm, contacts a tracker or enables public DHT. All sockets, including
the range web server, stay on loopback inside a Docker container with
`--network none`. Do not run the lab-enabled engine on Windows.

## Reviewed inputs

- Existing Rust/Debian image:
  `sha256:b36c246742b4d323472588f789601be34eb4af053255e2af53969078ee8dcec7`.
- Debian bookworm `libtorrent-rasterbar2.0` and `python3-libtorrent`
  `2.0.8-1+b1`, plus `libboost-python1.74.0` `1.74.0+ds1-21`.
  Use the official Debian signed package metadata and SHA-256 records. The
  development run reused previously verified packages after a clean Malwarebytes
  scan. Extract them only into the disposable lab, not the host.
- The repository's locked, previously reviewed Cargo dependency sources.
- For the old-client comparison, the torrent engine source at commit
  `a05b1aa8723c8d357f3bedc715ebae71f25e584b` (3.18.1).

Nothing is downloaded by this test script. Prepare and review missing inputs
separately before running them.

## Run

Create a disposable container named `de-torrent-v2-lab` and volume
`de-torrent-v2-work` at `/work`, with no network, all capabilities dropped,
`no-new-privileges`, 4 CPUs, 4 GiB RAM and 256 PIDs. Mount only the repository's
`torrent`, `vendor`, and approved Cargo source directories read-only at
`/input/torrent`, `/input/vendor`, and `/input/registry`. Do not mount a real
profile, home directory, Docker socket or credentials.

Copy `tests/torrent-binding/build-lab.sh` to `/work/build-lab.sh`; it builds the
engine offline with the test-only `lab` feature. Build the old torrent checkout
with the same locked inputs and preserve its executable at `/work/v1-engine`.
Then build the current engine with `/work/build-lab.sh`; the current executable
must be `/work/target/debug/de-torrent`.

Extract the three approved Debian archives with `dpkg-deb -x` into
`/work/peer-runtime`. Copy `interop.py` to `/work/interop.py`, then run:

```sh
cd /work/project/torrent
CARGO_HOME=/work/cargo cargo test --offline --locked --features lab \
  --config /work/source-config.toml --target-dir /work/target --lib -- --test-threads=2
LD_LIBRARY_PATH=/work/peer-runtime/usr/lib/x86_64-linux-gnu \
PYTHONPATH=/work/peer-runtime/usr/lib/python3/dist-packages \
python3 /work/interop.py
```

The script creates a new `/work/v2-tests/<timestamp>` directory per run and
writes `results.json`. Success requires every assertion to pass and exit code
zero; an incomplete JSON report is not a successful run.

Checks cover v1/v2/hybrid file and magnet transfers, both transfer directions,
v1-only peers and btih-only hybrid magnets, selected files, empty files, short
final blocks, 512-hash proofs, required MSE over TCP/uTP, range web seeds,
pause/resume, abrupt process loss, verified recovery and Stop all. Completed
files must match source SHA-256 hashes exactly. Malformed layers, version,
paths and conflicting hybrid layouts must be rejected. The Rust suite also
checks proof tampering, canonical bencode, full hash identities, cancellation
limits and saved-state invalidation.

## Clean up

Copy only the test reports and logs into `artifacts/torrent-v2-20261108`.
Remove this lab's container and volume after it stops, and remove the temporary
package copies and baseline archive. This deletes generated files, profiles,
private source snapshots and compiled lab binaries. Keep unrelated containers,
images, volumes, firewall rules, router settings and user downloads unchanged.

These checks establish isolated interoperability; they do not test public
tracker availability, private tracker policies or real-world VPN services.
