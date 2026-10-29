-- Offline provider lab ONLY; not yet importable into the app's Scripting page.
-- Run: cargo run --offline --locked --example lua_provider_lab
assert(xd.provider_api == "experimental-0", "Use the offline provider lab")

local function describe(context)
    -- The lab returns local fixture metadata here. No HTTP request is made.
    -- A future adapter could map an authorized service reply to the same plan.
    return xd.metadata(context.asset)
end

return {
    resolve = function(context)
        return describe(context)
    end,

    refresh = function(context, previous)
        -- A source reference may change; content/key/script identity must not.
        -- Rust independently checks that before reusing any completed chunks.
        return describe(context)
    end,

    transform = function(bytes, offset, plan)
        -- Binary string, zero-based byte offset. Each chunk is independent.
        -- The key ID grants use of one host-owned AES key, not a filesystem path.
        -- Replace this function with your own same-length transformation.
        return xd.aes_ctr(plan.key_id, plan.counter, bytes, offset)
    end,

    finish = function(plan, output)
        assert(output.size == plan.size, "Wrong output length")
        assert(output.sha256 == plan.output_sha256, "Wrong plaintext hash")
        -- Rust also checks the expected hash before publishing the output.
        return true
    end,
}
