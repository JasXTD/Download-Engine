-- Media API 1 / provider API 1, Download Engine 3.11.0 or newer.
-- Adapt the metadata to files you own. Size/hash placeholders deliberately fail
-- validation. Ordered MP4 segments must include the initialization bytes first.
assert(xd.media_api == 1, "This provider needs Download Engine 3.11.0 or newer")

local function resolve(request)
    return xd.media_plan({
        content_id = "my-media-revision-1",
        container = "mkv", -- "mp4" is also supported
        tracks = {
            {
                id = "video", kind = "video", format = "mp4", default = true,
                segments = {
                    { id = "init", url = xd.url_join(request.url, "video-init.mp4"),
                      size = 0, processing = "copy" }, -- enter actual byte count
                    { id = "part-1", url = xd.url_join(request.url, "video-1.enc"),
                      size = 0, processing = "lua",
                      input_sha256 = "REPLACE_WITH_ENCRYPTED_SEGMENT_SHA256",
                      output_sha256 = "REPLACE_WITH_DECRYPTED_SEGMENT_SHA256" },
                },
            },
            {
                id = "audio-en", kind = "audio", format = "aac",
                language = "eng", title = "English", default = true,
                segments = {
                    { id = "part-1", url = xd.url_join(request.url, "audio-1.aac"),
                      size = 0, processing = "copy" },
                },
            },
        },
    })
end

return {
    resolve = resolve,
    -- Refresh links but keep content_id, track/segment identities and all other
    -- metadata unchanged. Previous contains no URLs. It is nil on the first run.
    refresh = function(request, previous) return resolve(request) end,
    transform = function(context, bytes, offset)
        -- Called only for processing="lua". offset is relative to THIS segment;
        -- context.track_offset is its position in the concatenated track.
        -- PUBLIC TEST KEY. In an extension use xd.key_setting("content_key");
        -- in a personal script use xd.key with a protected token.
        local key = "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4"
        local counter = "000102030405060708090a0b0c0d0e0f"
        -- This example encrypts each segment independently. For continuous CTR
        -- use context.track_offset + offset and the track's matching counter.
        return xd.aes_ctr(key, counter, bytes, offset)
    end,
    finish = function(plan, result)
        assert(result.stage == "before_mux")
        return true
    end,
}
