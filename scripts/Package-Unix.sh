#!/usr/bin/env bash
# Build on the target OS. No downloads, system installation, or profile changes.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
output=${1:?Usage: bash scripts/Package-Unix.sh OUTPUT_DIRECTORY}
node "$root/scripts/Check-ReleaseDocs.cjs" "$root"
mkdir -p "$output"
output=$(cd "$output" && pwd)
host=$(rustc -vV | sed -n 's/^host: //p')
case "$host" in
  x86_64-unknown-linux-gnu) platform=linux-x64 ;;
  aarch64-unknown-linux-gnu) platform=linux-arm64 ;;
  x86_64-apple-darwin) platform=macos-x64 ;;
  aarch64-apple-darwin) platform=macos-arm64 ;;
  *) printf 'Unsupported native packaging host: %s\n' "$host" >&2; exit 1 ;;
esac
version=$(sed -n 's/^version = "\([0-9.]*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Invalid release version' >&2; exit 1; }
name="DownloadEngine-$version-$platform"
test ! -e "$output/$name" || { echo 'Package folder already exists; choose a fresh output directory.' >&2; exit 1; }
work=$(mktemp -d "$output/.native-package.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

# Copy only reviewed source files, never ignored fixtures or personal files that
# happen to be inside a documentation/extension folder. JSON::PP ships with Perl.
copy_source_tree() {
  local prefix=$1 destination=$2 relative suffix source component count=0
  local listed="$work/source-files"
  perl -MJSON::PP -e '
    my ($inventory, $prefix) = @ARGV;
    open my $fh, "<", $inventory or die "$inventory: $!";
    my $doc = decode_json(do { local $/; <$fh> });
    ref($doc->{files}) eq "ARRAY" or die "Missing source inventory files";
    my @files = grep { !ref($_) && index($_, "$prefix/") == 0 } @{$doc->{files}};
    @files or die "No reviewed source files under $prefix";
    for (@files) {
      m{\A[A-Za-z0-9_.+/-]+\z} && !m{(?:\A|/)\.{1,2}(?:/|\z)}
        or die "Invalid source inventory path";
    }
    print "$_\n" for @files;
  ' "$root/scripts/release-source-inventory.json" "$prefix" > "$listed" || return
  while IFS= read -r relative; do
    source="$root"
    local parts=()
    IFS=/ read -r -a parts <<< "$relative"
    for component in "${parts[@]}"; do
      source="$source/$component"
      [[ ! -L "$source" ]] || { echo "Source symlink refused: $relative" >&2; return 1; }
    done
    [[ -f "$source" ]] || { echo "Missing source file: $relative" >&2; return 1; }
    suffix=${relative#"$prefix"/}
    mkdir -p "$(dirname "$destination/$suffix")" || return
    cp "$source" "$destination/$suffix" || return
    count=$((count + 1))
  done < "$listed"
  [[ "$count" -gt 0 ]]
}

copy_media_build_record() {
  local source=$1 destination=$2 record
  [[ -d "$source" && ! -L "$source" ]] || { echo 'Invalid native build record directory' >&2; return 1; }
  mkdir -p "$destination" || return
  # Never copy the whole directory: FFmpeg's config.log dumps the environment,
  # and callers may supply a helper built before that log was excluded.
  for record in binaries.sha256 compiler-version.log config_components.h config.h config.mak ffmpeg-version.log hdr-metadata-test.log x264-config.mak; do
    [[ -f "$source/$record" && ! -L "$source/$record" ]] || { echo "Missing or linked native build record: $record" >&2; return 1; }
    cp "$source/$record" "$destination/$record" || return
  done
}

media=${DOWNLOAD_ENGINE_NATIVE_FFMPEG:-"$work/media"}
if [[ -z "${DOWNLOAD_ENGINE_NATIVE_FFMPEG:-}" ]]; then
  bash "$root/scripts/Build-UnixFfmpeg.sh" "$media"
fi
hash() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d ' ' -f 1; else shasum -a 256 "$1" | cut -d ' ' -f 1; fi; }
expected=$(awk '/  ffmpeg$/ {print $1}' "$media/build-record/binaries.sha256")
[[ "$expected" =~ ^[a-f0-9]{64}$ && "$(hash "$media/ffmpeg")" == "$expected" ]] || { echo 'Native FFmpeg checksum mismatch' >&2; exit 1; }
probe_expected=$(awk '/  ffprobe$/ {print $1}' "$media/build-record/binaries.sha256")
[[ "$probe_expected" =~ ^[a-f0-9]{64}$ && "$(hash "$media/ffprobe")" == "$probe_expected" ]] || { echo 'Native FFprobe checksum mismatch' >&2; exit 1; }
export DOWNLOAD_ENGINE_FFMPEG_SHA256="$expected"
cd "$root"
for variant in chromium firefox; do
  extension_version=$(sed -n 's/.*"version": *"\([0-9.]*\)".*/\1/p' "extension/manifest.$variant.json")
  [[ "$extension_version" == "$version" ]] || { echo 'Extension version mismatch' >&2; exit 1; }
done
cargo build --release --locked --offline --bins -j "${BUILD_JOBS:-2}"
target=${CARGO_TARGET_DIR:-"$root/target"}
case "$target" in /*) ;; *) target="$root/$target" ;; esac
stage="$work/$name"
if [[ "$platform" == macos-* ]]; then
  app="$stage/Download Engine.app"
  programs="$app/Contents/MacOS"
  resources="$app/Contents/Resources"
else
  programs="$stage"
  resources="$stage"
fi
mkdir -p "$programs" "$resources/tools/ffmpeg" "$resources/docs/licenses"
for program in de de-cli de-host de-torrent; do
  install -m 755 "$target/release/$program" "$programs/$program"
done
install -m 755 "$media/ffmpeg" "$resources/tools/ffmpeg/ffmpeg"
cp "$media/LICENSE" "$resources/tools/ffmpeg/"
cp -R "$media/licenses" "$resources/tools/ffmpeg/"
copy_media_build_record "$media/build-record" "$resources/tools/ffmpeg/build-record"
copy_source_tree tools/ffmpeg/source "$resources/tools/ffmpeg/source"
mkdir -p "$resources/scripts"
cp "$root/scripts/Build-UnixFfmpeg.sh" "$resources/scripts/"
cp "$root/tools/ffmpeg/PROVENANCE.md" "$resources/tools/ffmpeg/"
for file in README.md SECURITY.md PRIVACY.md CHANGELOG.md LICENSE THIRD-PARTY-NOTICES.md; do cp "$root/$file" "$resources/"; done
mkdir -p "$resources/torrent"
cp "$root/torrent/README.md" "$resources/torrent/"
for file in user-guide.md native-guide.md cross-platform.md automation.md extensions.md debrid-plugins.md first-extension.md mcp.md mcp-ai-guide.md lua-providers.md lua-media-providers.md lua-provider-core.md lua-provider-lab.md browser-release.md mega.md wetransfer.md reference.md localization.md releases.md github-release.md youtube-port.md; do cp "$root/docs/$file" "$resources/docs/"; done
copy_source_tree docs/images "$resources/docs/images"
copy_source_tree docs/locales "$resources/docs/locales"
copy_source_tree examples "$resources/examples"
node "$root/scripts/prepare_bundled_plugins.cjs" "$resources/plugins" "$programs/de-cli" "$media/ffmpeg"
copy_source_tree docs/licenses/common "$resources/docs/licenses/common"
for file in lua-automation-LICENSES Rust-standard-library.html yt-dlp-LICENSE; do cp "$root/docs/licenses/$file" "$resources/docs/licenses/"; done

# An isolated container may provide a separately verified native license bundle.
licenses=${DOWNLOAD_ENGINE_NATIVE_LICENSES:-"$work/licenses"}
if [[ -z "${DOWNLOAD_ENGINE_NATIVE_LICENSES:-}" ]]; then
  node "$root/scripts/Build-LicenseBundle.cjs" --target "$host" --output-root "$licenses"
fi
grep -Fq "\"target\": \"$host\"" "$licenses/docs/licenses/rust-dependencies.json" || { echo 'License bundle target mismatch' >&2; exit 1; }
cp -R "$licenses/docs/licenses/." "$resources/docs/licenses/"
for browser in chrome edge brave firefox; do
  destination="$resources/extensions/$browser"
  mkdir -p "$destination"
  for file in i18n-catalog.js i18n.js core.js capture.js torrent-links.js link-recovery.js page-links.js browser-login.js browser-video-relay.js video-button-manager.js video-button.js github-download.js media.js media-diagnostics.js dailymotion.js permission-flow.js login.html login.js background.js popup.html popup.css popup.js links.html links.css links.js; do cp "$root/extension/$file" "$destination/"; done
  copy_source_tree extension/_locales "$destination/_locales"
  copy_source_tree extension/icons "$destination/icons"
  cp "$root/LICENSE" "$root/PRIVACY.md" "$destination/"
  variant=chromium
  if [[ "$browser" == firefox ]]; then variant=firefox; fi
  cp "$root/extension/manifest.$variant.json" "$destination/manifest.json"
done

if [[ "$platform" == macos-* ]]; then
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Download Engine</string>
<key>CFBundleDisplayName</key><string>Download Engine</string>
<key>CFBundleIdentifier</key><string>com.xetro.downloadengine</string>
<key>CFBundleExecutable</key><string>de</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$version</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleIconFile</key><string>DownloadEngine</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
<key>UTImportedTypeDeclarations</key><array><dict>
<key>UTTypeIdentifier</key><string>org.bittorrent.torrent</string>
<key>UTTypeDescription</key><string>BitTorrent metadata</string>
<key>UTTypeConformsTo</key><array><string>public.data</string></array>
<key>UTTypeTagSpecification</key><dict>
<key>public.filename-extension</key><array><string>torrent</string></array>
<key>public.mime-type</key><string>application/x-bittorrent</string>
</dict></dict></array>
<key>CFBundleDocumentTypes</key><array><dict>
<key>CFBundleTypeName</key><string>BitTorrent file</string>
<key>CFBundleTypeRole</key><string>Viewer</string>
<key>LSHandlerRank</key><string>Alternate</string>
<key>CFBundleTypeExtensions</key><array><string>torrent</string></array>
<key>LSItemContentTypes</key><array><string>org.bittorrent.torrent</string></array>
</dict></array>
<key>CFBundleURLTypes</key><array><dict>
<key>CFBundleURLName</key><string>BitTorrent magnet link</string>
<key>CFBundleTypeRole</key><string>Viewer</string>
<key>CFBundleURLSchemes</key><array><string>magnet</string></array>
</dict></array>
</dict></plist>
PLIST
  /usr/bin/plutil -lint "$app/Contents/Info.plist"
  iconset="$work/DownloadEngine.iconset"
  mkdir "$iconset"
  for size in 16 32 128 256 512; do
    /usr/bin/sips -z "$size" "$size" "$root/assets/download-engine-master.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    /usr/bin/sips -z "$((size * 2))" "$((size * 2))" "$root/assets/download-engine-master.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
  done
  /usr/bin/iconutil -c icns "$iconset" -o "$resources/DownloadEngine.icns"
  # Ad-hoc signing validates the bundle locally; it is not notarization or an
  # Apple Developer signature. Do not disable Gatekeeper or clear quarantine.
  # Sign nested tools before the containing bundle. Signing its main executable
  # first also signs the bundle and fails when an Intel helper is still unsigned.
  # The bundle signing operation below signs de itself.
  for program in de-cli de-host de-torrent; do
    /usr/bin/codesign --force --sign - "$programs/$program"
  done
  /usr/bin/codesign --verify "$resources/tools/ffmpeg/ffmpeg"
  /usr/bin/codesign --force --sign - "$app"
  /usr/bin/codesign --verify --deep --strict "$app"
  /usr/bin/ditto -c -k --keepParent "$app" "$output/$name.zip"
  archive="$output/$name.zip"
else
  install -m 755 "$root/scripts/Install-LinuxDesktop.sh" "$stage/install-desktop.sh"
  cp "$root/assets/download-engine.png" "$stage/download-engine.png"
  tar -czf "$output/$name.tar.gz" -C "$work" "$name"
  archive="$output/$name.tar.gz"
fi
printf '%s  %s\n' "$(hash "$archive")" "$(basename "$archive")" > "$archive.sha256"
mv "$stage" "$output/$name"
printf 'Packaged %s\n' "$archive"
