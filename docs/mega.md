# MEGA public file downloads

On Windows, paste a complete `https://mega.nz/file/FILE_ID#KEY` link into Add
download, import it as a batch, or send it with the browser connector. Legacy
`https://mega.nz/#!FILE_ID!KEY` links are also accepted. The connector must be
reloaded after updating its files so it retains the link's decryption key.

The filename and size load when the download starts. A name supplied in Add
download is preserved. The existing download folder selection, connection
settings, speed limits, data quotas, queue controls, and segment display apply.
Larger files use multiple connections up to the configured limit. Small files
can use one connection.

Partial range files are encrypted. Pause/resume reuses those bytes and obtains
a fresh server address from the stable public link. After all ranges arrive,
the app decrypts the data locally and checks MEGA's file MAC. Only verified
plaintext is published as a completed download; an optional user SHA-256 is
checked as well. Decryption restarts after a pause during verification. Allow
disk space for both encrypted partial data and the decrypted file until completion.

The link's key is never sent to the MEGA API or file server. The complete share
link is retained in local download history and included when you explicitly
export links, so treat that history and exported links as access to the file.

Supported scope is individual public files. Folder links, account login,
password-protected share links, multi-server RAID responses, remote ZIP preview,
and synchronization checks are not supported. To use a different share link,
add a new download. A MEGA quota response stops the attempt with an explanation;
resume after the quota resets. No quota bypass or proxy rotation is performed.
Normal configured proxy routes are used. Jobs from a public-only site reader
retain that reader's existing Direct connection restriction.

Implementation uses Windows CNG AES, the MEGA API, and the app's existing range
workers and verified file-publication path. It does not install, execute, or
embed MegaBasterd. Protocol references:

- [MEGA SDK cryptography](https://github.com/meganz/sdk/blob/master/src/crypto/cryptopp.cpp)
- [MEGA API command implementation](https://github.com/meganz/sdk/blob/master/src/commands.cpp)
- [Windows CNG symmetric keys](https://learn.microsoft.com/en-us/windows/win32/api/bcrypt/nf-bcrypt-bcryptgeneratesymmetrickey)

Regression coverage uses the local HTTP fixture in `src/mega/tests.rs` and
independent deterministic vectors from Node's built-in cryptography library.
The vector generator is `tests/fixtures/mega/generate.cjs`; it needs no network
or packages. These checks do not establish live MEGA service compatibility.
