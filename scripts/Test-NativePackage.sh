#!/usr/bin/env bash
# Run only synthetic checks in a disposable profile. No browser registration,
# default-handler changes, real downloads or desktop preferences are involved.
set -euo pipefail
package=${1:?Usage: Test-NativePackage.sh EXTRACTED_PACKAGE_DIRECTORY}
package=$(cd "$package" && pwd)
case "$(uname -s)" in
  Darwin) programs="$package/Download Engine.app/Contents/MacOS"; resources="$package/Download Engine.app/Contents/Resources" ;;
  Linux) programs="$package"; resources="$package" ;;
  *) echo 'Unsupported native test host' >&2; exit 1 ;;
esac
fixture=$(mktemp -d "${TMPDIR:-/tmp}/de-package-smoke.XXXXXXXX")
trap 'rm -rf -- "$fixture"' EXIT
if [[ "$(uname -s)" == Darwin ]]; then
  plist="$package/Download Engine.app/Contents/Info.plist"
  read_plist() { /usr/libexec/PlistBuddy -c "Print $1" "$plist"; }
  [[ "$(read_plist :UTImportedTypeDeclarations:0:UTTypeIdentifier)" == org.bittorrent.torrent ]]
  [[ "$(read_plist :UTImportedTypeDeclarations:0:UTTypeConformsTo:0)" == public.data ]]
  [[ "$(read_plist :UTImportedTypeDeclarations:0:UTTypeTagSpecification:public.filename-extension:0)" == torrent ]]
  [[ "$(read_plist :UTImportedTypeDeclarations:0:UTTypeTagSpecification:public.mime-type)" == application/x-bittorrent ]]
  [[ "$(read_plist :CFBundleDocumentTypes:0:LSItemContentTypes:0)" == org.bittorrent.torrent ]]
  [[ "$(read_plist :CFBundleURLTypes:0:CFBundleURLSchemes:0)" == magnet ]]
  for program in de-cli de-host de-torrent; do
    /usr/bin/codesign --verify --strict "$programs/$program"
  done
  /usr/bin/codesign --verify --deep --strict "$package/Download Engine.app"
fi
expected_ffmpeg=$(awk '/  ffmpeg$/ {print $1}' "$resources/tools/ffmpeg/build-record/binaries.sha256")
if command -v sha256sum >/dev/null; then
  actual_ffmpeg=$(sha256sum "$resources/tools/ffmpeg/ffmpeg" | cut -d ' ' -f 1)
else
  actual_ffmpeg=$(shasum -a 256 "$resources/tools/ffmpeg/ffmpeg" | cut -d ' ' -f 1)
fi
[[ "$expected_ffmpeg" =~ ^[a-f0-9]{64}$ && "$actual_ffmpeg" == "$expected_ffmpeg" ]] || {
  echo 'Packaged FFmpeg differs from the checksum compiled into the app' >&2; exit 1;
}
export DOWNLOAD_ENGINE_HOME="$fixture/profile"
"$programs/de-cli" --version
"$programs/de-cli" list --no-start > "$fixture/list.json"
perl -MJSON::PP -e 'local $/; my $j=decode_json(<>); $j->{ok} && ref($j->{result}) eq "ARRAY" && @{$j->{result}}==0 or die "Unexpected CLI reply"' "$fixture/list.json"
perl -e '$j=q({"action":"ping"}); print pack("V",length($j)), $j' > "$fixture/ping"
"$programs/de-host" < "$fixture/ping" > "$fixture/reply"
perl -MJSON::PP -e '
  open my $f,"<",$ARGV[0] or die $!; binmode $f; local $/; my $b=<$f>;
  length($b)>4 && unpack("V",substr($b,0,4))==length($b)-4 or die "Bad frame";
  my $j=decode_json(substr($b,4));
  $j->{ok} && grep($_ eq "torrent_review_v1",@{$j->{capabilities}}) or die "Bad host reply";
' "$fixture/reply"
if [[ "${DOWNLOAD_ENGINE_SKIP_FFMPEG_TESTS:-0}" == 1 ]]; then
  printf 'FFmpeg encode/decode tests skipped as requested; bundle presence checked.\n'
else
  "$resources/tools/ffmpeg/ffmpeg" -hide_banner -nostdin -loglevel error \
    -f lavfi -i color=c=green:s=64x64:r=1 -frames:v 1 -c:v libx264 "$fixture/harmless.mp4"
  "$resources/tools/ffmpeg/ffmpeg" -hide_banner -nostdin -loglevel error -xerror \
    -i "$fixture/harmless.mp4" -f null -
  printf 'Packaged FFmpeg encode/decode passed.\n'
fi
test -x "$programs/de" && test -x "$programs/de-torrent"
test -s "$resources/tools/ffmpeg/source/ffmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8.tar.gz"
test -s "$resources/tools/ffmpeg/LICENSE"
test ! -e "$resources/tools/ffmpeg/build-record/config.log"
test -s "$resources/docs/licenses/RUST-DEPENDENCIES.md"
test -s "$resources/extensions/firefox/manifest.json"
# Package-Unix.sh copies a fixed list of documents. Every relative Markdown link,
# image and HTML src/href in the packaged README and reference guide must open
# inside the package.
perl -e '
  my ($root, @documents) = @ARGV;
  my @missing;
  for my $document (@documents) {
    open my $f, "<", "$root/$document" or do { push @missing, "$document: $!"; next };
    my $fenced = 0;
    while (my $line = <$f>) {
      if ($line =~ /^\s*(?:```|~~~)/) { $fenced = !$fenced; next }
      next if $fenced;
      for my $target ($line =~ /\]\(\s*<?([^)\s>]+)/g, $line =~ /\b(?:src|href)\s*=\s*"([^"]+)"/g) {
        next if $target =~ m{\A(?:#|[A-Za-z][A-Za-z0-9+.-]*:|//)};
        (my $path = $target) =~ s/[?#].*//s;
        $path =~ s/%([0-9A-Fa-f]{2})/chr hex $1/ge;
        my @parts = split m{/}, $document;
        pop @parts;
        my $inside = 1;
        for my $part (split m{/}, $path) {
          if ($part eq "..") { if (@parts) { pop @parts } else { $inside = 0 } }
          elsif ($part ne "" && $part ne ".") { push @parts, $part }
        }
        push @missing, "$document:$.: $target" unless $inside && -e join("/", $root, @parts);
      }
    }
  }
  die "Packaged documents link to missing files:\n", map("  $_\n", @missing) if @missing;
' "$resources" README.md docs/reference.md
printf 'Packaged CLI, native host, bundle contents, notices, document links and source passed.\n'
