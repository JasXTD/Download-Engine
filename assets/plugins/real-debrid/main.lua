local h = require("common")
local origin = "https://api.real-debrid.com"
-- Real-Debrid's documented public client for open-source device authorization.
-- This identifier is not a credential. User-bound secrets are obtained at login.
local public_client = "X245A4XAIBGVM"
local grant = "http://oauth.net/grant_type/device/1.0"
local errors = {
  [8] = "Account access expired or was revoked. Connect again.",
  [9] = "Account permission was denied. Check the Real-Debrid website.",
  [14] = "Real-Debrid has locked this account. Contact its support.",
  [15] = "Activate your account on Real-Debrid first.",
  [16] = "Real-Debrid does not support this host.",
  [17] = "This host is under maintenance.", [18] = "Your host limit has been reached.",
  [19] = "This host is temporarily unavailable.", [20] = "This link requires a Real-Debrid premium account.",
  [21] = "Too many active downloads. Wait for one to finish.",
  [22] = "Real-Debrid does not allow this IP address. Check your proxy or VPN with the service.",
  [23] = "Your traffic allowance has been used.", [24] = "This file is no longer available.",
  [28] = "The service does not allow this file.", [34] = "Too many requests. Try again later.",
  [35] = "This file was blocked by the service.", [36] = "The service's fair-use limit has been reached.",
}
local function checked(data, status)
  assert(status < 400 and not data.error,
    errors[data.error_code] or "Real-Debrid refused this request. Check your account and host availability on its website.")
  return data
end
local function exchange(account, code)
  local data, status = h.request(h.session(origin), origin .. "/oauth/v2/token", {
    client_id = account.client_id, client_secret = account.client_secret, code = code, grant_type = grant,
  })
  checked(data, status)
  account.access_token = h.text(data.access_token, "access token")
  account.refresh_token = h.text(data.refresh_token, "refresh token")
  return account
end
local action = xd.extension.job
if action == "disconnect" then return h.disconnect() end
if action == "connect" then
  local session = h.session(origin)
  local device, status = h.request(session, origin .. "/oauth/v2/device/code?client_id=" .. public_client .. "&new_credentials=yes")
  checked(device, status)
  h.text(device.device_code, "device code")
  h.text(device.user_code, "approval code", 32)
  assert(device.user_code:match("^[%w%-]+$"), "Invalid approval code.")
  assert(type(device.interval) == "number" and device.interval >= 0 and device.interval <= 60
    and type(device.expires_in) == "number" and device.expires_in > 0, "Invalid device authorization lifetime.")
  local interval = math.max(5, math.ceil(device.interval))
  xd.log("info", "Open https://real-debrid.com/device and enter code: " .. device.user_code)
  for _ = 1, math.min(36, math.floor(math.min(180, device.expires_in) / interval)) do
    xd.sleep(interval * 1000)
    local credentials, code = h.request(session, origin .. "/oauth/v2/device/credentials?client_id=" .. public_client .. "&code=" .. xd.url_encode(device.device_code))
    if code == 200 and credentials.client_id and credentials.client_secret then
      local account = exchange({ client_id = h.text(credentials.client_id, "client ID"),
        client_secret = h.text(credentials.client_secret, "client secret") }, device.device_code)
      h.save(account)
      return h.status("Real Debrid connected. Use Check account to see your plan.")
    end
    -- The public credentials endpoint also returns HTTP 403 with both error
    -- fields null before approval (checked against the live API). xd.json maps
    -- those null fields to nil. Only this empty response or explicit pending
    -- may wait again; refusals and other errors still stop immediately.
    local pending = credentials.error == "authorization_pending"
      or (code == 403 and next(credentials) == nil)
    assert(pending, "Device approval was refused or expired. Connect again.")
  end
  error("Approval timed out. Click Connect account to get a new code.")
end
local account = h.account()
h.text(account.access_token, "saved access token")
h.text(account.refresh_token, "saved refresh token")
h.text(account.client_id, "saved client ID")
h.text(account.client_secret, "saved client secret")
local function request(path, fields)
  local data, status = h.request(h.session(origin, account.access_token), origin .. "/rest/1.0/" .. path, fields)
  if status == 401 or data.error_code == 8 then
    account = exchange(account, account.refresh_token)
    -- Persist rotated credentials before retrying the request. Never retry
    -- arbitrary server errors: generating a link may have a remote side effect.
    h.save(account)
    data, status = h.request(h.session(origin, account.access_token), origin .. "/rest/1.0/" .. path, fields)
  end
  return checked(data, status)
end
if action == "account" then
  local user = request("user")
  return h.status(user.type == "premium" and "Real Debrid connected — premium account."
    or "Real Debrid connected — free account; host restrictions apply.")
end
assert(action == "generate", "Unknown plugin action.")
local data = request("unrestrict/link", { link = event.url, remote = 0 })
local rows = data.download and { data } or data
local items = {}
for _, row in ipairs(rows) do
  assert(#items < 1000, "The service returned too many links.")
  items[#items + 1] = h.item(row.download, row.filename, row.filesize, { "real-debrid.com" })
end
assert(#items > 0, "Real-Debrid did not return a downloadable file.")
-- One connection satisfies the service's per-link chunks limit, including 1.
return { title = "Real Debrid — generated downloads", single_connection = true, items = items }
