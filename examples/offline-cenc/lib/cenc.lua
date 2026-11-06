-- Bounded original Lua CENC teaching parser. See README for the supported subset.
-- Offsets are zero-based. Unsupported layouts fail rather than guessing.
local M = {}
local LIMIT = 256 * 1024
local function hex(s)
    return (s:gsub('.', function(c) return string.format('%02x', c:byte()) end))
end
local function read(input, offset, count)
    assert(count >= 0 and count <= LIMIT and offset >= 0 and offset + count <= input:size(),
        'CENC read outside segment')
    local value = input:read(offset, count)
    assert(#value == count, 'Truncated CENC input')
    return value
end
local function uint(input, offset, width)
    return string.unpack('>I' .. width, read(input, offset, width))
end
local function boxes(input, start, finish)
    local result = {}
    while start < finish do
        assert(#result < 4096 and finish - start >= 8, 'Invalid CENC box list')
        local size, kind = uint(input, start, 4), read(input, start + 4, 4)
        assert(size >= 8 and size <= finish - start, 'Invalid or unsupported CENC box size')
        result[#result + 1] = {start = start, stop = start + size, kind = kind}
        start = start + size
    end
    assert(start == finish, 'Truncated CENC box list')
    return result
end
local function one(list, name)
    local found
    for _, box in ipairs(list) do
        if box.kind == name then
            assert(not found, 'Duplicate CENC ' .. name)
            found = box
        end
    end
    return assert(found, 'Missing CENC ' .. name)
end
local function children(input, box, skip)
    return boxes(input, box.start + 8 + (skip or 0), box.stop)
end
local function path(input, box, ...)
    for _, name in ipairs({...}) do box = one(children(input, box), name) end
    return box
end
local function free(edits, box)
    edits[#edits + 1] = {offset = box.start + 4, size = 4, value = 'free'}
end
local function version(input, box, flags)
    assert(uint(input, box.start + 8, 4) == flags, 'Unsupported CENC ' .. box.kind .. ' version/flags')
end
function M.string_input(bytes)
    return {size = function() return #bytes end,
        read = function(_, offset, count) return bytes:sub(offset + 1, offset + count) end}
end
function M.init(input)
    local top = boxes(input, 0, input:size())
    local moov = one(top, 'moov')
    local track = one(children(input, moov), 'trak')
    local tkhd = one(children(input, track), 'tkhd')
    assert(uint(input, tkhd.start + 8, 1) == 0, 'Unsupported tkhd version')
    local id = uint(input, tkhd.start + 20, 4)
    local stsd = path(input, track, 'mdia', 'minf', 'stbl', 'stsd')
    version(input, stsd, 0)
    assert(uint(input, stsd.start + 12, 4) == 1, 'One sample description is required')
    local entries = children(input, stsd, 8)
    assert(#entries == 1, 'Invalid sample description count')
    local entry = entries[1]
    local video = entry.kind == 'encv'
    assert(video or entry.kind == 'enca', 'Expected encrypted sample entry')
    if not video then
        assert(uint(input, entry.start + 16, 2) == 0, 'Unsupported audio sample entry version')
    end
    local sinf = one(boxes(input, entry.start + (video and 86 or 36), entry.stop), 'sinf')
    local nested = children(input, sinf)
    local frma, schm = one(nested, 'frma'), one(nested, 'schm')
    assert(frma.stop == frma.start + 12, 'Invalid original sample format')
    local format = read(input, frma.start + 8, 4)
    assert(format == (video and 'avc1' or 'mp4a'), 'This example supports AVC and AAC only')
    version(input, schm, 0)
    assert(schm.stop == schm.start + 20 and read(input, schm.start + 12, 4) == 'cenc'
        and uint(input, schm.start + 16, 4) == 65536, 'Only cenc AES-CTR is supported')
    local tenc = path(input, one(nested, 'schi'), 'tenc')
    version(input, tenc, 0)
    assert(tenc.stop == tenc.start + 32 and uint(input, tenc.start + 12, 2) == 0
        and uint(input, tenc.start + 14, 1) == 1, 'Unsupported tenc protection')
    local iv_size = uint(input, tenc.start + 15, 1)
    assert(iv_size == 8 or iv_size == 16, 'Unsupported CENC IV size')
    local kid = hex(read(input, tenc.start + 16, 16))
    local edits = {{offset = entry.start + 4, size = 4, value = format}}
    free(edits, sinf)
    for _, box in ipairs(children(input, moov)) do
        if box.kind == 'pssh' then free(edits, box) end
    end
    for _, box in ipairs(top) do if box.kind == 'pssh' then free(edits, box) end end
    return {id = id, iv_size = iv_size, kid = kid, edits = edits}
end
local function fragment(input, state, key)
    local top = boxes(input, 0, input:size())
    local moof, mdat = one(top, 'moof'), one(top, 'mdat')
    local traf = one(children(input, moof), 'traf')
    local list = children(input, traf)
    for _, box in ipairs(list) do
        assert(box.kind ~= 'sgpd' and box.kind ~= 'sbgp' and box.kind ~= 'uuid',
            'Sample groups and alternate encryption layouts are unsupported')
    end
    local tfhd, trun = one(list, 'tfhd'), one(list, 'trun')
    local header = uint(input, tfhd.start + 8, 4)
    assert(header & ~0x020038 == 0 and header & 0x020000 ~= 0,
        'tfhd must use default-base-is-moof without overrides')
    assert(uint(input, tfhd.start + 12, 4) == state.id, 'CENC track ID changed')
    local cursor, default_size = tfhd.start + 16, 0
    if header & 8 ~= 0 then cursor = cursor + 4 end
    if header & 16 ~= 0 then default_size = uint(input, cursor, 4); cursor = cursor + 4 end
    if header & 32 ~= 0 then cursor = cursor + 4 end
    assert(cursor == tfhd.stop, 'Invalid tfhd length')
    local flags = uint(input, trun.start + 8, 4)
    assert(flags & ~0x01000F05 == 0 and flags & 1 ~= 0, 'Unsupported trun flags')
    assert(not (flags & 4 ~= 0 and flags & 0x400 ~= 0), 'Conflicting trun sample flags')
    local count = uint(input, trun.start + 12, 4)
    assert(count > 0 and count <= 65536, 'Invalid CENC sample count')
    local offset = string.unpack('>i4', read(input, trun.start + 16, 4)) + moof.start
    assert(offset >= mdat.start + 8 and offset <= mdat.stop, 'CENC samples outside mdat')
    local positions, sizes = {}, {}
    cursor = trun.start + 20 + (flags & 4 ~= 0 and 4 or 0)
    for i = 1, count do
        if flags & 0x100 ~= 0 then cursor = cursor + 4 end
        local size = default_size
        if flags & 0x200 ~= 0 then size = uint(input, cursor, 4); cursor = cursor + 4 end
        if flags & 0x400 ~= 0 then cursor = cursor + 4 end
        if flags & 0x800 ~= 0 then cursor = cursor + 4 end
        assert(cursor <= trun.stop and size > 0 and offset + size <= mdat.stop, 'Invalid CENC sample bounds')
        positions[i], sizes[i], offset = offset, size, offset + size
    end
    assert(cursor == trun.stop and offset == mdat.stop, 'Unsupported trailing sample data')
    local senc, saiz, saio = one(list, 'senc'), one(list, 'saiz'), one(list, 'saio')
    local encryption_flags = uint(input, senc.start + 8, 4)
    assert(encryption_flags == 0 or encryption_flags == 2, 'Unsupported senc flags')
    assert(uint(input, senc.start + 12, 4) == count, 'CENC sample count mismatch')
    version(input, saiz, 0); version(input, saio, 0)
    assert(uint(input, saiz.start + 13, 4) == count, 'CENC auxiliary count mismatch')
    local default_aux = uint(input, saiz.start + 12, 1)
    assert(saiz.stop == saiz.start + 17 + (default_aux == 0 and count or 0), 'Invalid saiz length')
    assert(saio.stop == saio.start + 20 and uint(input, saio.start + 12, 4) == 1
        and uint(input, saio.start + 16, 4) == senc.start + 16 - moof.start, 'Invalid CENC auxiliary offset')
    local edits = {}
    free(edits, senc); free(edits, saiz); free(edits, saio)
    cursor = senc.start + 16
    for i = 1, count do
        local begin = cursor
        assert(cursor + state.iv_size <= senc.stop, 'Truncated CENC IV')
        local iv = read(input, cursor, state.iv_size) .. string.rep('\0', 16 - state.iv_size)
        cursor = cursor + state.iv_size
        local parts = 1
        if encryption_flags == 2 then
            assert(cursor + 2 <= senc.stop, 'Truncated subsample count')
            parts = uint(input, cursor, 2); cursor = cursor + 2
            assert(parts > 0 and parts <= 8192 and cursor + parts * 6 <= senc.stop,
                'Invalid CENC subsample list')
        end
        local used, encrypted_offset = 0, 0
        for _ = 1, parts do
            local clear, encrypted = 0, sizes[i]
            if encryption_flags == 2 then
                clear, encrypted = uint(input, cursor, 2), uint(input, cursor + 2, 4)
                cursor = cursor + 6
            end
            assert(clear + encrypted > 0 and used + clear + encrypted <= sizes[i], 'CENC subsample outside sample')
            if encrypted > 0 then
                assert(#edits < 131072, 'Too many CENC encrypted ranges')
                edits[#edits + 1] = {offset = positions[i] + used + clear, size = encrypted,
                    key = key, iv = hex(iv), cipher_offset = encrypted_offset}
            end
            used, encrypted_offset = used + clear + encrypted, encrypted_offset + encrypted
        end
        assert(used == sizes[i], 'CENC subsamples do not cover sample')
        local aux = default_aux > 0 and default_aux or uint(input, saiz.start + 16 + i, 1)
        assert(cursor <= senc.stop and cursor - begin == aux, 'CENC auxiliary size mismatch')
    end
    assert(cursor == senc.stop, 'Trailing CENC encryption records')
    return edits
end
function M.write(input, output, edits)
    table.sort(edits, function(a, b) return a.offset < b.offset end)
    local cursor = 0
    local function copy(stop)
        while cursor < stop do
            local count = math.min(LIMIT, stop - cursor)
            output:write(read(input, cursor, count)); cursor = cursor + count
        end
    end
    for _, edit in ipairs(edits) do
        assert(edit.offset >= cursor and edit.offset + edit.size <= input:size(), 'Overlapping CENC edits')
        copy(edit.offset)
        if edit.value then
            assert(#edit.value == edit.size, 'CENC edit changed length')
            output:write(edit.value); cursor = cursor + edit.size
        else
            local offset = 0
            while offset < edit.size do
                local count = math.min(LIMIT, edit.size - offset)
                output:write(xd.aes_ctr(edit.key, edit.iv, read(input, cursor, count), edit.cipher_offset + offset))
                offset, cursor = offset + count, cursor + count
            end
        end
    end
    copy(input:size())
    return true
end
function M.transform(input, output, state, key, initialization)
    return M.write(input, output, initialization and M.init(input).edits or fragment(input, state, key))
end
return M
