-- Provider API 1. Lua constructs CTR and XORs bytes; Rust supplies AES blocks.
-- Replace both hashes with trusted metadata for your own source.
assert(xd.provider_api == 1)
local key = "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4" -- PUBLIC TEST KEY
-- For private keys, initialize key = xd.key(protected_token) only when
-- xd.phase == "processor". See docs/lua-providers.md; resolver locals are separate.
local initial = "000102030405060708090a0b0c0d0e0f"
local function describe(request)
    return {
        url = request.url, size = 2097189,
        input_sha256 = "REPLACE_WITH_CIPHERTEXT_SHA256",
        output_sha256 = "REPLACE_WITH_PLAINTEXT_SHA256",
        transform_id = "lua-ctr-example-v1",
    }
end
local function add(counter, amount)
    for i = 16, 1, -1 do
        local n = counter[i] + amount % 256
        counter[i] = n % 256
        amount = amount // 256 + n // 256
    end
    assert(amount == 0, "Counter overflow")
end
local function transform(plan, bytes, offset)
    if #bytes == 0 then return "" end
    local counter = {}
    for i = 1, 32, 2 do counter[#counter+1] = assert(tonumber(initial:sub(i,i+1),16)) end
    add(counter, offset // 16)
    local skip = offset % 16
    local count = (#bytes + skip + 15) // 16
    local blocks = {}
    for i = 1, count do
        blocks[i] = string.char(table.unpack(counter))
        if i < count then add(counter, 1) end
    end
    local stream = xd.aes_blocks(key, table.concat(blocks))
    local out = {}
    for i = 1, #bytes do
        out[i] = string.char(bytes:byte(i) ~ stream:byte(skip+i))
    end
    return table.concat(out)
end
return {
    resolve = describe,
    refresh = function(request, previous) return describe(request) end,
    transform = transform,
    finish = function(plan, result)
        assert(result.size == plan.size and result.sha256 == plan.output_sha256)
        return true
    end,
}
