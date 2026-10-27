# Project website and Google Search Console

The public website is <https://jasxtd.github.io/Download-Engine/>.
It is a static GitHub Pages site published from `main`, folder `/docs`.

## Website files

- `docs/index.html`: page content, download links, canonical URL, social metadata
  and SoftwareApplication structured data.
- `docs/site.css`: responsive styling, focus states and reduced-motion support.
- `docs/images/`: existing app logo and reviewed screenshots with example data.
- `docs/.nojekyll`: publish the HTML and assets directly, without Jekyll processing.
- `docs/sitemap.xml`: the canonical public homepage.

No package installation, JavaScript application build, external font, analytics
script or browser storage is needed. Native HTML handles navigation and the FAQ.
The page remains usable with JavaScript disabled.

The Web Grabber illustration uses native radio controls to preview file discovery
and offline website capture. Its filenames, sizes and site tree are example
content, not a live scan or a desktop screenshot. Keep that label visible.

The hosting section documents MEGA public files, public WeTransfer transfers and
exposed-link discovery, including GitHub raw-file handling. It clearly marks these
as **development-source features not included in v3.1.1**. Keep the availability
notice, FAQ, download-section note and README consistent until a release contains
them. Do not imply universal hosting support or full offline functionality for
interactive websites. Service-specific limitations are in `mega.md`,
`wetransfer.md` and `user-guide.md`.

## Updating the website

Edit the files, review the desktop and mobile layouts, and push to `main`.
GitHub Pages deploys the `/docs` contents automatically. Check the repository's
Actions page or Settings → Pages for the deployment result, then verify the
published page and affected assets.

Featured packages are explicitly pinned to **3.1.1**. When a new release is
published, update the version labels, seven asset URLs, release-notes URL and
SoftwareApplication version/download URL together. Verify each asset exists in
that release. The separate **Latest GitHub release** link always opens GitHub's
current latest release. The website never downloads or runs an installer
automatically.

For design changes, check narrow and wide layouts, both grabber-preview modes,
keyboard navigation, FAQ controls, reduced motion and operation without
JavaScript. Verify all seven release assets and the documentation anchors.

Review new files explicitly and add their paths to
`scripts/release-source-inventory.json`. Website-only updates do not replace
existing versioned application release assets.

## Connect Google Search Console

1. Open [Google Search Console](https://search.google.com/search-console/) and sign
   in yourself with the Google account you want to manage the website.
2. Select **Add property**, then **URL prefix**. Enter exactly:
   `https://jasxtd.github.io/Download-Engine/`.
   Keep the HTTPS scheme, capital letters in the path and trailing slash.
3. Under the verification methods, expand **HTML tag** and copy the complete
   `<meta name="google-site-verification" content="…">` tag that Google provides.
   This is a public website-verification value, not your account password. Do not
   send passwords, sign-in codes or account cookies to anyone.
4. Have the maintainer insert that exact tag into the `<head>` of
   `docs/index.html`, commit and push it. No token is prefilled in this repository.
   Wait for GitHub Pages to finish deploying, then confirm the public homepage
   contains the tag. Return to Search Console and click **Verify**.
5. Open **Sitemaps** for the verified property. Submit
   `https://jasxtd.github.io/Download-Engine/sitemap.xml` (or `sitemap.xml` if the
   interface already shows the property prefix).
6. Use **URL inspection** for `https://jasxtd.github.io/Download-Engine/`.
   Run **Test live URL**, then **Request indexing** when available.
7. Leave the verification tag in place. Google periodically rechecks ownership.
   Use the indexing and performance reports to monitor coverage and search queries.

This property covers the GitHub Pages website. It does not verify ownership of
`github.com` or grant Search Console control over the repository's GitHub page.
Links between the website and repository help visitors and crawlers discover both.

Crawling can take days to weeks. An indexing request or a sitemap does not
guarantee inclusion or a particular search position; repeatedly submitting the
same URL does not speed crawling up.

Official guidance:

- [Google: verify site ownership](https://support.google.com/webmasters/answer/9008080)
- [Google: request crawling and indexing](https://developers.google.com/search/docs/crawling-indexing/ask-google-to-recrawl)
- [GitHub: configure a Pages publishing source](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site)
