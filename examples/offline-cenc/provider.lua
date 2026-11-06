-- No service URLs, browser credentials or native modules. catalog.lua is generated.
local cenc = require('lib.cenc')
local license = require('lib.license')
local catalog = require('catalog')
assert(xd.supports('media.stream_processing'), 'Download Engine 3.15.0 is required')
local states = {}
local function raw(hex)
    return (hex:gsub('..', function(pair) return string.char(tonumber(pair, 16)) end))
end
local function resolve(request, previous)
    local origin = request.url:match('^(http://127%.0%.0%.1:%d+)/sample%-cenc/?$')
    assert(origin, 'Use the offline sample-cenc URL from the fixture harness')
    local plan = xd.json(catalog.plan)
    if previous then assert(previous.content_id == plan.content_id, 'Offline fixture changed') end
    local expected = {}
    for _, kid in ipairs(catalog.kids) do expected[#expected + 1] = raw(kid) end
    -- The harness supplies only fresh public test material. A service adapter
    -- should replace these two inputs with protected RSA/binary settings and
    -- declare its real API/CDN hosts; protected extensions cannot return loopback.
    local credentials = require('fixture_credentials')
    local keys = license.open(xd.rsa_private_key(credentials.identity), credentials.license,
        plan.content_id, raw(catalog.nonce), expected)
    for kid, key in pairs(keys) do xd.handoff_key(kid, key) end
    for _, track in ipairs(plan.tracks) do
        for _, segment in ipairs(track.segments) do segment.url = origin .. segment.url end
    end
    return xd.media_plan(plan)
end
return {
    resolve = resolve, refresh = resolve,
    prepare_track = function(context, bytes)
        states[context.track_id] = cenc.init(cenc.string_input(bytes))
        return true
    end,
    transform_stream = function(context, input, output)
        local state = assert(states[context.track_id], 'Missing CENC initialization')
        return cenc.transform(input, output, state, xd.session_key(state.kid), context.segment_index == 0)
    end,
    transform = function() error('This example uses streaming CENC processing') end,
    finish = function(plan, result)
        assert(result.stage == 'before_mux' and result.tracks == #plan.tracks)
        return true
    end,
}
