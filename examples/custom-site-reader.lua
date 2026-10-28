-- Scripting > Scripts > Loaded scripts > New script > Trigger: Site reader.
-- Set Pages to your public site's host/path; test its URL before enabling.
-- Change a.download to match the site's HTML. No JavaScript runs here.
assert(xd.api_version == 1, "This example requires Lua API 1")
local page = xd.fetch(event.url)
assert(page.status == 200, "Page returned HTTP " .. page.status)
local items = {}
for _, href in ipairs(xd.html_select(page.body, "a.download[href]", "href")) do
    items[#items + 1] = {
        kind = "file",
        url = xd.url_join(page.url, href),
    }
end
assert(#items > 0, "No download links matched the selector")
return { title = xd.html_select(page.body, "title")[1], items = items }
