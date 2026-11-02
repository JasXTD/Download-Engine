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
"$resources/tools/ffmpeg/ffmpeg" -hide_banner -nostdin -loglevel error \
  -f lavfi -i color=c=green:s=64x64:r=1 -frames:v 1 -c:v libx264 "$fixture/harmless.mp4"
"$resources/tools/ffmpeg/ffmpeg" -hide_banner -nostdin -loglevel error -xerror \
  -i "$fixture/harmless.mp4" -f null -
test -x "$programs/de" && test -x "$programs/de-torrent"
test -s "$resources/tools/ffmpeg/source/ffmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8.tar.gz"
test -s "$resources/tools/ffmpeg/LICENSE"
test ! -e "$resources/tools/ffmpeg/build-record/config.log"
test -s "$resources/docs/licenses/RUST-DEPENDENCIES.md"
test -s "$resources/extensions/firefox/manifest.json"
printf 'Packaged CLI, native host, FFmpeg encode/decode, notices and source passed.\n'
