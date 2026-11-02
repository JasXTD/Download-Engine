# objc2 workspace notices

The locked crates published from <https://github.com/madsmtm/objc2> omit some
workspace license files. Download Engine includes their checksum-verified,
unmodified `.crate` source archives alongside these supplemental notices.

`MIT-objc2` is the byte-for-byte MIT notice, including Steven Sheldon's copyright,
from the upstream revision `4fc083f1c6d6784577e38b0ee8dbd344481e2fd2`:
<https://github.com/madsmtm/objc2/blob/4fc083f1c6d6784577e38b0ee8dbd344481e2fd2/LICENSE.txt>.
Its Git blob is `e9ab3babc0d8602b83dccde8a462f8f3dc1e52ad` and SHA-256 is
`e353f37b12aefbb9f9b29490e837cfee05d9bda70804b3562839a3285c1df1e5`.
The same blob occurs at revisions `5237cbf081d04ef75a1f6a207b4018e7c1ae7438`
and `e282618be4c3a3b9542957e0c8540e9588472ce8`.

Later workspace revisions instead provide `LICENSE.md`, explaining the license
choices and Apple SDK provenance without reproducing the full license texts.
The included `objc2-LICENSE.md` is byte-for-byte from revision
`7b1abfd750a2cacaea71d6a56ecfb83cb7de560b`:
<https://github.com/madsmtm/objc2/blob/7b1abfd750a2cacaea71d6a56ecfb83cb7de560b/LICENSE.md>.
Its Git blob is `540ae4c81adb5f64a9f61e222b688880da24300e` and SHA-256 is
`7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54`.
The same blob occurs at the locked revisions
`8852b424193ca41602281b3d7540d7c8ed51e49a`,
`8d214f5477365ffcbcbb7de058c86ed9a518efb7`,
`b4167b582b2f75f9a1be75495c41b765344fd03c`, and
`d7d2fa23ceaa5e6096c923b081040e5d81b3b9df`.

For crates still declared MIT-only, the original MIT copyright notice is retained
with the newer workspace explanation and all original source notices. For the
newer frameworks and dispatch2 that explicitly offer Apache-2.0 as an alternative,
Download Engine selects Apache-2.0 and includes its complete text plus the workspace
explanation. The generated dependency inventory records each selection.

The packaging helper checks the exact crate version, repository, declared license,
and `.cargo_vcs_info.json` revision before using these files. Unknown versions
require another review. License files are read as data; packaging executes no
commands from them and downloads nothing.

The older `dispatch` 0.2.0 crate from
<https://github.com/SSheldon/rust-dispatch/tree/82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748>
also omits license text, both in its archive and that repository revision. Its
published Cargo manifest declares MIT and names Steven Sheldon as its sole
author. The bundle supplies the same standard MIT text and his copyright notice
above, preserving the entire original archive. This is a supplemental notice,
not a claim that the dispatch repository contained that license file.
