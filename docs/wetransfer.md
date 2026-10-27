# WeTransfer downloads

In Download Engine, paste a public `https://we.tl/...` short link or a
`https://wetransfer.com/downloads/...` transfer link into Add URL. Link imports,
browser Send link, the CLI and local automation use the same download path.
WeTransfer support does not require an extension change.

The app requests the entire transfer. It saves the file or archive supplied by
WeTransfer, usually a ZIP for multiple files, without extracting or running it.
The filename loads when the transfer starts. A filename you entered is preserved;
otherwise a server-provided filename takes precedence over transfer metadata.

The normal queue controls, connection settings, speed limits, quotas, progress
display and optional SHA-256 verification apply. Multiple connections and reuse
of partial files require HTTP byte ranges, a known size and a strong ETag from
the file server. Some generated archives cannot provide these, so they use one
connection and start from the beginning after a pause or interrupted request.

The original share link stays in the queue. Retry and resume resolve a fresh file
address. Saved parts are reused only when the transfer identity, byte count and
strong ETag match; changed content requires Restart. Expiration of the transfer
itself cannot be fixed by refreshing its file address: ask the sender for a new
transfer and add it as a new download.

Supported scope is anonymous public transfers, including recipient links that do
not require identity verification. Password-protected transfers, account login,
email verification, Collect boards, selecting individual files, remote ZIP
preview and synchronization checks are not supported. If WeTransfer requires a
browser challenge, open the transfer in your browser to check access.

Anonymous session cookies and CSRF tokens are held only for the lookup and sent
only to that transfer page's API origin. Browser cookies, saved app credentials
and personal information are not used. Signed file addresses and session tokens
are not stored in job history. The share link itself remains in local history
and explicit link exports; anyone holding it may be able to access the transfer.

All requests use HTTPS and the configured proxy route. Share redirects remain
within recognized WeTransfer link hosts; file redirects are limited to WeTransfer
domains and its documented S3 storage buckets. Site-reader jobs retain their
public-only DNS checks and Direct connection requirement. Metadata responses,
redirect counts and lookup time are bounded. Error pages are not saved as files.

This is an independent implementation of the web transfer protocol, using
Download Engine's existing HTTP transfer pipeline. No third-party downloader,
external script or new dependency is installed or executed. WeTransfer may change
its web endpoints. Protocol and service references:

- [WeTransfer download guide](https://wetransfer.com/help-center/how-to/download-a-transfer)
- [WeTransfer network domains](https://wetransfer.com/help-center/troubleshooting/configure-firewall)
- [transferwee protocol reference](https://github.com/iamleot/transferwee/blob/master/transferwee.py)
- [JDownloader transfer metadata reference](https://github.com/mirror/jdownloader/blob/master/src/jd/plugins/decrypter/WeTransferComFolder.java)

Regression tests in `src/wetransfer/tests.rs` use an isolated HTTP fixture for
resolution, anonymous sessions, parallel transfers, cancellation, address refresh,
changed-file rejection and error paths. An isolated live check on 27 October 2026
also completed a user-supplied 15-byte single-file transfer and verified its saved
size and SHA-256. Large transfers and multi-file bundles have not been checked
against the live service.
