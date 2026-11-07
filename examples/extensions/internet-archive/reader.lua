-- Site reader: lists the files in an Internet Archive item.
--
-- It runs when you open a page such as https://archive.org/details/ITEM with
-- Add URL, Find videos on this page, or the browser extension's button. It
-- returns a list of files, and Download Engine shows that list so you can
-- choose which ones to download. It downloads nothing by itself.

-- 1. Read the item's name from the page address. event.url is the page.
--    https://archive.org/details/ITEM/anything-else  ->  ITEM
local item_name = event.url:match("^https?://archive%.org/details/([%w%._%-]+)")
assert(item_name, "Open an archive.org item page, such as archive.org/details/ITEM")

-- 2. Ask archive.org's public metadata service about the item.
--    xd.fetch reaches only the hosts listed under "network" in manifest.json.
local reply = xd.fetch("https://archive.org/metadata/" .. item_name)
assert(reply.status == 200, "archive.org answered with HTTP " .. reply.status)
local item = xd.json(reply.body)
assert(item.files, "archive.org has no files for " .. item_name)

-- 3. Read a setting. You can change it on the extension's Settings tab.
local include_copies = xd.setting("include_derivatives")

-- A file can sit in a folder inside the item, such as "disc1/track01.mp3".
-- Encode each part of its path on its own, so the "/" between them stays.
local function file_address(path)
  local parts = {}
  for part in path:gmatch("[^/]+") do
    parts[#parts + 1] = xd.url_encode(part)
  end
  return "https://archive.org/download/" .. item_name .. "/" .. table.concat(parts, "/")
end

-- 4. Turn archive.org's file list into the list Download Engine expects.
--    "original" files were uploaded. "derivative" files are copies archive.org
--    made from them. "metadata" files describe the item, so they are skipped.
local files = {}
for _, file in ipairs(item.files) do
  local wanted = file.source == "original"
    or (include_copies and file.source == "derivative")
  -- Files marked private can't be downloaded, such as books that are only
  -- lent out. They are left out of the list.
  local private = file.private == "true"
  -- Stream playlists need a different kind of item, so this example skips them.
  local name = file.name:lower()
  local playlist = name:match("%.m3u8$") or name:match("%.mpd$")
  if wanted and not private and not playlist then
    -- archive.org gives sizes as text. Download Engine takes a number above 0,
    -- so an empty or unknown size is left out.
    local size = tonumber(file.size)
    files[#files + 1] = {
      kind = "file",
      url = file_address(file.name),
      filename = file.name:match("[^/]+$"),
      label = file.format,
      size = size and size > 0 and size or nil,
    }
  end
end
assert(#files > 0, "This item has no files that can be downloaded")

-- Download Engine accepts up to 1,000 files from one page.
if #files > 1000 then
  xd.log("warning", "Showing the first 1,000 of " .. #files .. " files")
  for index = #files, 1001, -1 do
    files[index] = nil
  end
end

-- 5. Return the item's title and its files. Some items list several titles.
local title = item.metadata and item.metadata.title
if type(title) == "table" then
  title = title[1]
end
return { title = title or item_name, items = files }
