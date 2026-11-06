-- Original teaching protocol, not Widevine or any service's license format.
-- Only generated test identities and keys are used by the accompanying harness.
local M = {}
local function hex(s)
    return (s:gsub('.', function(c) return string.format('%02x', c:byte()) end))
end
local function same(a, b)
    if #a ~= #b then return false end
    local difference = 0
    for i = 1, #a do difference = difference | (a:byte(i) ~ b:byte(i)) end
    return difference == 0
end
function M.request(key, content, kids, nonce)
    assert(#nonce == 32 and #kids > 0 and #kids <= 16, 'Invalid fixture request')
    local parts = {'DECR1', nonce, xd.digest('sha256', content), string.pack('>I2', #kids)}
    local seen = {}
    for _, kid in ipairs(kids) do
        assert(#kid == 16 and not seen[kid], 'Invalid fixture key ID')
        seen[kid], parts[#parts + 1] = true, kid
    end
    local body = table.concat(parts)
    local signature = xd.rsa_sign(key, 'pss-sha256', body)
    return body .. string.pack('>I2', #signature) .. signature
end
function M.open(key, response, content, nonce, expected)
    assert(#response <= 65536, 'Fixture license too large')
    local p = 1
    local function take(n)
        assert(n >= 0 and p + n - 1 <= #response, 'Truncated fixture license')
        local value = response:sub(p, p + n - 1)
        p = p + n
        return value
    end
    assert(take(5) == 'DECL1', 'Wrong fixture license format')
    assert(same(take(32), nonce), 'Fixture request nonce mismatch')
    assert(same(take(32), xd.digest('sha256', content)), 'Fixture content mismatch')
    local wrapped = take(string.unpack('>I2', take(2)))
    local iv = take(16)
    local encrypted = take(string.unpack('>I4', take(4)))
    local signed = response:sub(1, p - 1)
    local mac = take(32)
    assert(p == #response + 1, 'Trailing fixture license bytes')
    local session = xd.rsa_decrypt(key, 'oaep-sha256', wrapped)
    assert(#session == 32, 'Invalid fixture session key')
    assert(same(mac, xd.hmac('sha256', session, signed)), 'Fixture license authentication failed')
    local plain = xd.aes_ctr(hex(session), hex(iv), encrypted, 0)
    assert(#plain >= 2, 'Truncated fixture keys')
    local count = string.unpack('>I2', plain)
    assert(count == #expected and #plain == 2 + count * 32, 'Fixture key count mismatch')
    local keys, wanted = {}, {}
    for _, kid in ipairs(expected) do
        assert(#kid == 16 and not wanted[hex(kid)], 'Invalid expected fixture key ID')
        wanted[hex(kid)] = true
    end
    for i = 0, count - 1 do
        local offset = 3 + i * 32
        local id = hex(plain:sub(offset, offset + 15))
        assert(wanted[id] and not keys[id], 'Unexpected or duplicate fixture key ID')
        keys[id] = plain:sub(offset + 16, offset + 31)
    end
    return keys
end
return M
