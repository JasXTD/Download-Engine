# Project website and Google Search Console

The public website is <https://jasxtd.github.io/Download-Engine/>.
It is a static GitHub Pages site published from `main`, folder `/docs`.

## Website files

- `docs/index.html`: page content, download links, canonical URL, social metadata
  and SoftwareApplication structured data.
- `docs/site.css`: responsive styling, focus states, animation and reduced-motion support.
- `docs/site.js`: scroll reveal, header state, current-section navigation and the
  hero theme preview.
- `docs/slideshow.js`: local screenshot rotation and playback controls.
- `docs/images/`: app logo, reviewed screenshots with example data, theme-art
  thumbnails and the 1200 × 630 `social-preview.jpg`.
- `docs/.nojekyll`: publish the HTML and assets directly, without Jekyll processing.
- `docs/sitemap.xml`: the canonical public homepage.

No package installation, application build, external font, analytics script or
browser storage is needed. Native HTML handles navigation and the FAQ. The small
local slideshow script rotates the browser screenshots every two seconds while
visible. Hover and hidden tabs suspend rotation; keyboard focus or manual image
selection pauses it until Play is chosen. Reduced-motion users start paused.
The first screenshot and the rest of the page remain usable without JavaScript.

Motion is CSS plus `site.js`, with no libraries. The hero copy fades in on load,
sections reveal as they scroll into view, and cards lift slightly on hover.
`site.js` adds the `motion` class only when JavaScript runs and reduced motion is
off; content that is already on screen is never hidden. The hero theme stack
rotates Aurora, Neon Courier, Reactor Night and Moonveil every 3.8 seconds while visible.
Hover, keyboard focus and hidden tabs suspend it; choosing a theme or Pause stops
it until Play is chosen. Reduced-motion users start paused, and the global
reduced-motion rule removes transitions and animations.

The hero and video section explicitly name YouTube, Bilibili, TikTok and Twitch.
HLS and DASH describe compatible stream formats from other sites, not universal
page support. Keep these descriptions and their per-service guide links aligned
with the support table in `README.md`; retain the public-video, quality, DRM and
live-recording limits.

The Web Grabber illustration uses native radio controls to preview file discovery
and offline website capture. Its filenames, sizes and site tree are example
content, not a live scan or a desktop screenshot. Keep that label visible.

The Lua section explains automation, site readers and download providers as three
roles in the same scripting workspace. Keep their capabilities aligned with
`automation.md` and `lua-providers.md`. The selectable code example only reads queue
state; it never executes in the website. Provider API 1 handles single files and
same-length transforms, and its encrypted fixtures do not establish support for
any streaming service. Link to the guides for full contracts and limits.

Web Grabber has separate file-discovery and static offline-capture workflows.
Saved projects and repeat schedules apply to file discovery, not resumable offline
website capture. Preserve the illustration's native radio controls and the links
to both guide sections.

The custom-themes section uses the same native radio technique to switch between
real 3.4.0 screenshots; the theme-collection panel around it is an illustration.
The public collection includes Aurora, Neon Courier, Reactor Night and Moonveil.
Use the current Moonveil artwork and captures when updating the fourth preset.
The theme screenshots replace the capture machine's local download path in the
details panel with `D:\Downloads\Programs`; redact any personal path the same way
before adding a new screenshot.

The hosting section documents MEGA public files, public WeTransfer transfers and
exposed-link discovery, including GitHub raw-file handling. These features have
shipped since **v3.2.0**. Keep the availability notice, FAQ, download-section note and README
consistent with the featured release. Do not imply universal hosting support or
full offline functionality for interactive websites. Service-specific limitations are in `mega.md`,
`wetransfer.md` and `user-guide.md`.

## Updating the website

Edit the files, review the desktop and mobile layouts, and push to `main`.
GitHub Pages deploys the `/docs` contents automatically. Check the repository's
Actions page or Settings → Pages for the deployment result, then verify the
published page and affected assets.

Featured packages are explicitly pinned to **3.5.2**. Publish the v3.5.2 GitHub
release with all seven assets before or together with pushing this site, or the
download buttons return 404. When a new release is
published, update the version labels, seven asset URLs, release-notes URL and
SoftwareApplication version/download URL together. Verify each asset exists in
that release. The separate **Latest GitHub release** link always opens GitHub's
current latest release. The website never downloads or runs an installer
automatically.

For design changes, check narrow and wide layouts, both grabber-preview modes,
all theme-preview choices, keyboard navigation, FAQ controls, reduced motion and
operation without JavaScript. Check slideshow and hero-theme timing, wraparound,
pause/play, image selection, visibility, loading failure and stable panel height.
At 375 px the page must not scroll horizontally. Verify all seven release
assets and the documentation anchors.

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
   `docs/index.html`, commit and push it.
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
