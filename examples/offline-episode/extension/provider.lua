-- Media API 1 example. No service login, CDM, DRM license or website access.
-- The builder supplies catalog.lua from media generated with the bundled FFmpeg.
assert(xd.media_api == 1, "This example needs Download Engine 3.11.0 or newer")
local upstream = require("lib.upstream")
local catalog = require("catalog")

local function locales(value, available, preferred, optional)
    local requested = {}
    for item in value:gmatch("[^,]+") do
        local locale = item:match("^%s*(.-)%s*$")
        assert(locale ~= "", "Enter locale names separated by commas")
        requested[#requested + 1] = locale
    end
    if optional and #requested == 1 and requested[1]:lower() == "none" then return {} end
    assert(#requested > 0, "Choose at least one locale, or NONE for subtitles")
    local choices, known, seen, result = {}, {}, {}, {}
    for locale in pairs(available) do choices[#choices + 1], known[locale] = locale, true end
    for _, locale in ipairs(upstream.expand_languages(requested, choices, preferred)) do
        assert(known[locale], "Sample locale unavailable: " .. locale .. "; choose ja-JP or en-US")
        -- The core requires unique tracks, including when a locale was typed twice.
        if not seen[locale] then result[#result + 1], seen[locale] = locale, true end
    end
    return result
end

local function source_origin(url)
    local port = url:match("^http://127%.0%.0%.1:(%d+)/sample%-episode/?$")
    assert(port and tonumber(port) >= 1 and tonumber(port) <= 65535,
        "Use the local sample-episode URL printed by Start-Server.ps1; this is an offline example")
    return "http://127.0.0.1:" .. port
end

local function track_plan(base, source, default)
    local track = {
        id = source.id, kind = source.kind, format = source.format,
        language = source.language, title = source.title, default = default,
        segments = {},
    }
    local function add(id, relative, part)
        track.segments[#track.segments + 1] = {
            id = id, url = base .. relative, size = part.size,
            processing = part.processing,
            input_sha256 = part.input_sha256, output_sha256 = part.output_sha256,
        }
    end
    if source.kind == "subtitle" then
        add("subtitles", source.path, source)
    else
        add("init", upstream.build_url("", source.id, source.initialization), source.init)
        for index, number in ipairs(upstream.expand_timeline(source.timeline, source.start_number)) do
            local part = assert(source.segments[index], "Sample metadata is incomplete")
            local relative = upstream.build_url("", source.id, source.media, number)
            add("part-" .. number, relative, part)
        end
    end
    return track
end

local function resolve(request)
    local base = source_origin(request.url)
    local video = assert(catalog.video[xd.setting("quality")], "Sample quality unavailable")
    local tracks = {track_plan(base, video, true)}
    for index, locale in ipairs(locales(xd.setting("audio_languages"), catalog.audio, "ja-JP", false)) do
        tracks[#tracks + 1] = track_plan(base, catalog.audio[locale], index == 1)
    end
    for index, locale in ipairs(locales(xd.setting("subtitle_languages"), catalog.subtitles, "en-US", true)) do
        tracks[#tracks + 1] = track_plan(base, catalog.subtitles[locale], index == 1)
    end
    return xd.media_plan({content_id = catalog.content_id, container = xd.setting("container"), tracks = tracks})
end

return {
    resolve = resolve,
    -- Settings are live when a worker starts. Reuse the saved selection instead
    -- of changing an in-progress download when the user edits their preferences.
    refresh = function(request, previous)
        local base = source_origin(request.url)
        assert(previous.content_id == catalog.content_id, "Sample content changed; restart this download")
        local tracks = {}
        for _, selected in ipairs(previous.tracks) do
            local source = assert(catalog.by_id[selected.id], "Saved sample track no longer exists")
            tracks[#tracks + 1] = track_plan(base, source, selected.default)
        end
        return xd.media_plan({content_id = catalog.content_id, container = previous.container, tracks = tracks})
    end,
    transform = function(context, bytes, offset)
        local source = assert(catalog.by_id[context.track_id], "Unknown sample track")
        assert(context.kind ~= "subtitle" and source.counter, "Unexpected encrypted sample")
        -- PUBLIC TEST KEY: intentionally reusable only with these generated samples.
        -- Private keys belong in an aes_key setting; they never enter the resolver.
        local key = "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4"
        if xd.setting("use_saved_key") then key = xd.key_setting("sample_key") end
        return xd.aes_ctr(key, source.counter, bytes, context.track_offset + offset)
    end,
    finish = function(plan, result)
        assert(result.stage == "before_mux" and result.tracks == #plan.tracks)
        local size = 0
        for _, track in ipairs(plan.tracks) do
            for _, segment in ipairs(track.segments) do size = size + segment.size end
        end
        assert(size == result.size, "Sample processing was incomplete")
        return true
    end,
}
