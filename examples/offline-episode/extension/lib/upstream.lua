-- Lua adaptations of requestsAll, expandLangs and buildUrl (download.go), and
-- expandTimeline (mpd.go), from CuteTenshii/crunchyroll-downloader, commit
-- 734421c9c1268bdd3716ba65d429ec54ae5f2358. See NOTICE.md in the demo bundle.
-- Changes: bounded finite timelines; copy before sorting; reject unsupported
-- templates. $Number$ retains upstream's five-digit formatting deliberately.
--[[
Copyright 2025-2026 CuteTenshii

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
]]
local M = {}

function M.requests_all(languages)
    for _, locale in ipairs(languages) do
        if locale:lower() == "all" then return true end
    end
    return false
end

function M.expand_languages(requested, available, preferred)
    if not M.requests_all(requested) then return requested end
    local selected, seen = {}, {}
    for _, locale in ipairs(requested) do
        if locale:lower() ~= "all" and not seen[locale] then
            selected[#selected + 1], seen[locale] = locale, true
        end
    end
    if #selected == 0 then
        for _, locale in ipairs(available) do
            if locale == preferred then
                selected[#selected + 1], seen[locale] = locale, true
                break
            end
        end
    end
    local sorted = {table.unpack(available)}
    table.sort(sorted)
    for _, locale in ipairs(sorted) do
        if not seen[locale] then
            selected[#selected + 1], seen[locale] = locale, true
        end
    end
    return selected
end

function M.expand_timeline(timeline, start_number)
    assert(math.type(start_number) == "integer" and start_number >= 0,
        "Sample start number must be a non-negative integer")
    local result, number = {}, start_number
    for _, entry in ipairs(timeline) do
        local repetitions = entry.r or 0
        assert(math.type(repetitions) == "integer" and repetitions >= 0,
            "This offline example supports only finite non-negative repeats")
        assert(#result + repetitions + 1 <= 2048, "Sample timeline is too large")
        for _ = 0, repetitions do
            result[#result + 1] = number
            number = number + 1
        end
    end
    return result
end

function M.build_url(base, representation, template, number)
    assert(representation:match("^[a-z0-9_-]+$"), "Invalid sample representation")
    if number ~= nil then
        assert(math.type(number) == "integer" and number >= 0 and number <= 99999,
            "Invalid sample segment number")
        local formatted = string.format("%05d", number)
        template = template:gsub("%$Number%$", formatted):gsub("%$Number%%05d%$", formatted)
    end
    template = template:gsub("%$RepresentationID%$", representation)
    assert(not template:find("$", 1, true), "Unsupported sample segment template")
    return base .. template
end

return M
