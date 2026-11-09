#!/usr/bin/env bash
# Runs only inside the disposable Debian CI container, never on the developer host.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == true && -f /source.tar && -d /ci-inputs ]] || { echo 'Expected isolated CI inputs' >&2; exit 1; }
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=2 BUILD_JOBS=2
export PATH="/opt/node/bin:$PATH"
case "${1:-}" in
  prepare)
    [[ "${DE_SOURCE_SHA256:-}" =~ ^[a-f0-9]{64}$ && "${DE_SOURCE_COMMIT:-}" =~ ^[a-f0-9]{40}$ ]]
    printf '%s  /source.tar\n' "$DE_SOURCE_SHA256" | sha256sum --check --status
    mkdir /source /output
    tar -xf /source.tar --no-same-owner -C /source
    while read -r expected size url extra; do
      [[ -z "$expected" || "$expected" == \#* ]] && continue
      [[ -z "$extra" && "$expected" =~ ^[a-f0-9]{64}$ && "$size" =~ ^[0-9]+$ ]]
      name=${url##*/}
      [[ "$name" =~ ^[A-Za-z0-9_+.~-]+$ ]]
      [[ $(stat -c %s "/ci-inputs/$name") == "$size" ]]
      printf '%s  %s\n' "$expected" "/ci-inputs/$name" | sha256sum --check --status
    done < /source/scripts/linux-ci-inputs.lock
    for package in /ci-inputs/*.deb; do
      # Avoid maintainer scripts and service startup in the ephemeral container.
      dpkg-deb --fsys-tarfile "$package" | tar -xf - -C / --keep-directory-symlink --no-same-owner
    done
    ldconfig
    mkdir /opt/node
    tar -xJf /ci-inputs/node-v24.22.2-linux-x64.tar.xz --no-same-owner --strip-components=1 -C /opt/node node-v24.22.2-linux-x64/bin/node node-v24.22.2-linux-x64/LICENSE
    [[ $(node --version) == v24.22.2 ]]
    cd /source
    rustc -vV
    pkg-config --modversion openssl dbus-1
    cargo fetch --locked --target x86_64-unknown-linux-gnu
    cargo fetch --locked --manifest-path torrent/Cargo.toml --target x86_64-unknown-linux-gnu
    bash scripts/Build-UnixFfmpeg.sh /native-ffmpeg
    ;;
  test-package)
    cd /source
    export DOWNLOAD_ENGINE_NATIVE_FFMPEG=/native-ffmpeg DOWNLOAD_ENGINE_FFMPEG_PATH=/native-ffmpeg/ffmpeg
    DOWNLOAD_ENGINE_FFMPEG_SHA256=$(awk '/  ffmpeg$/ {print $1}' /native-ffmpeg/build-record/binaries.sha256)
    export DOWNLOAD_ENGINE_FFMPEG_SHA256
    fixture=$(mktemp -d /tmp/de-linux-ci.XXXXXXXX)
    keyring_pid=''
    cleanup() {
      if [[ -n "$keyring_pid" ]]; then kill "$keyring_pid" 2>/dev/null || true; wait "$keyring_pid" 2>/dev/null || true; fi
      case "$fixture" in /tmp/de-linux-ci.*) rm -rf -- "$fixture" ;; *) return 1 ;; esac
    }
    trap cleanup EXIT
    export XDG_DATA_HOME="$fixture/data" XDG_RUNTIME_DIR="$fixture/runtime" GNOME_KEYRING_CONTROL="$fixture/runtime/keyring"
    mkdir -m 700 "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR" "$GNOME_KEYRING_CONTROL"
    printf 'public-ci-fixture' | gnome-keyring-daemon --foreground --unlock --components=secrets --control-directory="$GNOME_KEYRING_CONTROL" > "$fixture/keyring.log" 2>&1 &
    keyring_pid=$!
    ready=0
    for _ in $(seq 1 50); do
      if dbus-send --session --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus.NameHasOwner string:org.freedesktop.secrets | grep -q 'boolean true'; then ready=1; break; fi
      sleep 0.1
    done
    [[ "$ready" == 1 ]] || { echo 'Isolated test keyring did not start' >&2; exit 1; }
    node --test scripts/Run-NativeCi.test.cjs scripts/LicenseNotices.test.cjs
    node --test tests/dailymotion.test.cjs tests/page_links.test.cjs tests/media.test.cjs tests/media_security.test.cjs tests/video_button.test.cjs tests/background.test.cjs tests/extension.test.cjs tests/capture.test.cjs tests/permission_flow.test.cjs tests/browser_login.test.cjs
    cargo test --locked --offline --manifest-path torrent/Cargo.toml -j 2 -- --test-threads=2
    cargo test --locked --offline --test hls --test browser_video -j 2 -- --test-threads=2
    cargo test --locked --offline --bin de desktop::hls::tests -j 2 -- --test-threads=2
    cargo test --locked --offline --bin de desktop::browser_video::tests -j 2 -- --test-threads=2
    bash scripts/Test-UnixPackaging.sh
    bash scripts/Package-Unix.sh /packages
    version=$(sed -n 's/^version = "\([0-9.]*\)"$/\1/p' Cargo.toml | head -n 1)
    [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
    name="DownloadEngine-$version-linux-x64.tar.gz"
    (cd /packages && sha256sum --check "$name.sha256")
    mkdir "$fixture/extracted"
    tar -xzf "/packages/$name" --no-same-owner -C "$fixture/extracted"
    bash scripts/Test-NativePackage.sh "$fixture/extracted/DownloadEngine-$version-linux-x64"
    for program in de de-cli de-host de-torrent; do cmp "target/release/$program" "$fixture/extracted/DownloadEngine-$version-linux-x64/$program"; done
    cp "/packages/$name" "/packages/$name.sha256" /output/
    node -e 'require("node:fs").writeFileSync("/output/linux-validation.json",JSON.stringify({commit:process.env.DE_SOURCE_COMMIT,version:process.argv[1],architecture:"x86_64",base:"Debian 12",testsPassed:true,extractedPackageSmokePassed:true,networkDisconnectedForTests:true,ffmpegBundled:true},null,2))' "$version"
    ;;
  *) echo 'Expected prepare or test-package' >&2; exit 1 ;;
esac
