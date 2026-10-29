-- Provider API 1: replace the sample metadata with your own trusted metadata.
-- Source URL is entered in Scripting. Rust owns files, downloading and resume.
assert(xd.provider_api == 1)
local function describe(request)
    return {
        url = request.url,
        size = 2097189,
        input_sha256 = "REPLACE_WITH_CIPHERTEXT_SHA256",
        output_sha256 = "REPLACE_WITH_PLAINTEXT_SHA256",
        transform_id = "my-aes-ctr-v1",
    }
end

return {
    resolve = describe,
    refresh = function(request, previous) return describe(request) end,
    transform = function(plan, bytes, offset)
        -- PUBLIC TEST KEY only. For private keys use xd.key(protected_token);
        -- create the token with de-cli scripts protect-key (see docs/lua-providers.md).
        local key = "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4"
        local counter = "000102030405060708090a0b0c0d0e0f"
        return xd.aes_ctr(key, counter, bytes, offset)
    end,
    finish = function(plan, result)
        assert(result.sha256 == plan.output_sha256)
        return true
    end,
}
