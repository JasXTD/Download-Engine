-- Site reader: lists the files attached to a GitHub release, so you can
-- choose which ones to download. It works on pages such as:
--   https://github.com/OWNER/REPO/releases/tag/v1.2.3   one release
--   https://github.com/OWNER/REPO/releases/latest       the newest release
--   https://github.com/OWNER/REPO/releases              also the newest release

-- 1. Read the owner, repository and the rest of the address.
local owner, repo, rest = event.url:match("^https?://github%.com/([^/]+)/([^/]+)/releases/?([^?#]*)")
assert(owner, "Open a GitHub releases page, such as github.com/OWNER/REPO/releases/latest")
rest = rest:gsub("/$", "") -- "latest/" means the same as "latest"

-- 2. Pick the GitHub API address for this page.
local api = "https://api.github.com/repos/" .. owner .. "/" .. repo .. "/releases/"
local tag = rest:match("^tag/(.+)$")
if tag then
  api = api .. "tags/" .. tag -- The tag is already encoded, as it was in the page address.
elseif rest == "" or rest == "latest" then
  api = api .. "latest"
else
  error("Open one release, such as .../releases/tag/v1.0, or .../releases/latest")
end

-- 3. Ask GitHub. Without signing in, its API answers about 60 requests an
--    hour from one address, and says 403 or 429 after that.
local reply = xd.fetch(api, { headers = { Accept = "application/vnd.github+json" } })
if reply.status == 403 or reply.status == 429 then
  error("GitHub is limiting requests right now. Try again in an hour")
end
assert(reply.status == 200, "GitHub answered with HTTP " .. reply.status)
local release = xd.json(reply.body)

-- 4. Each file attached to the release becomes one item in the list.
local files = {}
for _, asset in ipairs(release.assets or {}) do
  -- Stream playlists need a different kind of item, so this example skips them.
  local name = asset.name:lower()
  local playlist = name:match("%.m3u8$") or name:match("%.mpd$")
  if not playlist then
    files[#files + 1] = {
      kind = "file",
      url = asset.browser_download_url,
      filename = asset.name,
      -- Download Engine takes a size above 0, so an empty file's size is left out.
      size = asset.size > 0 and asset.size or nil,
    }
  end
end

-- 5. A setting you can change on the Settings tab. GitHub also makes source
--    code archives for every release; they are listed only if you ask.
if xd.setting("include_source") then
  local base = "https://github.com/" .. owner .. "/" .. repo .. "/archive/refs/tags/"
    .. xd.url_encode(release.tag_name)
  for _, format in ipairs({ "zip", "tar.gz" }) do
    files[#files + 1] = {
      kind = "file",
      url = base .. "." .. format,
      filename = repo .. "-" .. release.tag_name .. "." .. format,
      label = "Source code (" .. format .. ")",
    }
  end
end
assert(#files > 0, "This release has no files attached")

-- Download Engine accepts up to 1,000 files from one page.
if #files > 1000 then
  xd.log("warning", "Showing the first 1,000 of " .. #files .. " files")
  for index = #files, 1001, -1 do
    files[index] = nil
  end
end

-- 6. Use the release's name as the title, or its tag when it has no name.
local title = release.name
if not title or title == "" then
  title = release.tag_name
end
return { title = repo .. " " .. title, items = files }
