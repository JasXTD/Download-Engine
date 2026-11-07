-- Site reader: gets the original file behind a Wikimedia Commons file page,
-- such as https://commons.wikimedia.org/wiki/File:Example.jpg
--
-- A Commons page shows a smaller preview. This reader asks the public
-- MediaWiki API for the full-size original, and returns that one file.

-- 1. Take the page title from the address: everything after /wiki/.
local title = event.url:match("^https?://commons%.wikimedia%.org/wiki/([^?#]+)")
assert(title and title:match("^File:"), "Open a Commons file page, such as .../wiki/File:Example.jpg")

-- Addresses write some characters as codes: "Café" becomes "Caf%C3%A9".
-- Turn the codes back into characters before using the title.
title = title:gsub("%%(%x%x)", function(code)
  return string.char(tonumber(code, 16))
end)

-- 2. Ask the API for the original's address, size and type.
--    "%7C" is "|", the separator the API uses between the values we want.
local api = "https://commons.wikimedia.org/w/api.php?action=query&format=json&formatversion=2"
  .. "&prop=imageinfo&iiprop=url%7Csize%7Cmime&titles=" .. xd.url_encode(title)
local reply = xd.fetch(api)
assert(reply.status == 200, "Commons answered with HTTP " .. reply.status)

-- 3. Find the page in the answer. JSON null becomes nil in Lua.
local answer = xd.json(reply.body)
local page = answer.query and answer.query.pages and answer.query.pages[1]
assert(page and not page.missing and not page.invalid, "Commons has no file called " .. title)
local original = page.imageinfo and page.imageinfo[1]
assert(original and original.url, "Commons did not return the file's address")

-- 4. Return one file. Its name is the page title without "File:".
local filename = page.title:gsub("^File:", "")
return {
  title = page.title,
  items = {
    {
      kind = "file",
      url = original.url,
      filename = filename,
      label = original.mime,
      size = original.size,
    },
  },
}
