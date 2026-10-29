-- Offline lab ONLY. The counter construction and decryption are written in Lua.
-- Rust supplies only a granted AES key and its block-encryption primitive.
-- Run the already-built host with --script examples/lua-provider-custom-ctr.lua.
assert(xd.provider_api == "experimental-0", "Use the offline provider lab")

local function counter_bytes(hex)
    assert(#hex == 32, "A counter is 16 bytes")
    local bytes = {}
    for i = 1, 32, 2 do
        bytes[#bytes + 1] = assert(tonumber(hex:sub(i, i + 1), 16), "Invalid counter")
    end
    return bytes
end

local function add_counter(bytes, amount)
    for i = 16, 1, -1 do
        local n = bytes[i] + amount % 256
        bytes[i] = n % 256
        amount = amount // 256 + n // 256
    end
    assert(amount == 0, "Counter overflow")
end

local function transform(bytes, offset, plan)
    assert(offset >= 0 and math.type(offset) == "integer", "Invalid byte offset")
    if #bytes == 0 then return "" end
    local counter = counter_bytes(plan.counter)
    add_counter(counter, offset // 16)
    local skip = offset % 16
    local count = (#bytes + skip + 15) // 16
    local blocks = {}
    for i = 1, count do
        blocks[i] = string.char(table.unpack(counter))
        if i < count then add_counter(counter, 1) end
    end
    -- AES(counter) is the keystream; CTR decryption is ciphertext XOR keystream.
    -- A real format may lay out the counter differently: this sample uses all
    -- 128 bits, big endian. It is not a Widevine or container parser.
    local stream = xd.aes_blocks(plan.key_id, table.concat(blocks))
    local output = {}
    for i = 1, #bytes do
        output[i] = string.char(string.byte(bytes, i) ~ string.byte(stream, skip + i))
    end
    return table.concat(output)
end

return {
    resolve = function(context) return xd.metadata(context.asset) end,
    refresh = function(context, previous) return xd.metadata(context.asset) end,
    transform = transform,
    finish = function(plan, output)
        assert(output.size == plan.size and output.sha256 == plan.output_sha256,
            "Plaintext validation failed")
        return true
    end,
}
