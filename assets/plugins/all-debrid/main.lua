local h = require("common")
local origin = "https://api.alldebrid.com"
local errors = {
  AUTH_BAD_APIKEY = "Account access expired or was revoked. Connect again.",
  AUTH_MISSING_APIKEY = "Connect your account first.",
  AUTH_BLOCKED = "Approve this connection on the AllDebrid website, then try again.",
  AUTH_USER_BANNED = "AllDebrid has blocked this account. Contact its support.",
  PIN_EXPIRED = "The approval code expired. Click Connect account again.",
  PIN_INVALID = "The approval code is invalid. Click Connect account again.",
  LINK_HOST_NOT_SUPPORTED = "AllDebrid does not support this host.",
  LINK_HOST_UNAVAILABLE = "This host is temporarily unavailable on AllDebrid.",
  LINK_DOWN = "This file is no longer available.",
  LINK_PASS_PROTECTED = "This link requires a file password. Generate it on the service website.",
  LINK_HOST_FULL = "The host limit has been reached. Try again later.",
  LINK_TOO_MANY_DOWNLOADS = "Too many active downloads. Wait for a download to finish.",
  LINK_HOST_LIMIT_REACHED = "Your account's host allowance has been used.",
  LINK_HOST_NOT_SUPPORTED_FOR_FREE = "This host requires an AllDebrid premium account.",
}
local function request(session, path, fields)
  local data, status = h.request(session, origin .. path, fields)
  if data.status ~= "success" or status >= 400 then
    local code = type(data.error) == "table" and data.error.code or ""
    error(errors[code] or "AllDebrid refused this request. Check your account and host availability on its website.")
  end
  assert(type(data.data) == "table", "AllDebrid returned an incomplete response.")
  return data.data
end
local function account_status(session)
  local data = request(session, "/v4/user")
  assert(type(data.user) == "table", "AllDebrid returned incomplete account details.")
  return data.user.isPremium == true and "All Debrid connected — premium account." or "All Debrid connected — free account; host restrictions apply."
end
local action = xd.extension.job
if action == "disconnect" then return h.disconnect() end
if action == "connect" then
  local session = h.session(origin)
  local pin = request(session, "/v4.1/pin/get")
  h.text(pin.pin, "approval code", 32)
  assert(pin.pin:match("^[%w%-]+$"), "Invalid approval code.")
  h.text(pin.check, "approval check")
  assert(type(pin.expires_in) == "number" and pin.expires_in >= 5, "Approval code already expired.")
  xd.log("info", "Open https://alldebrid.com/pin/ and enter code: " .. pin.pin)
  -- Bounded polling stays below the worker's 50-request budget. Stop kills the
  -- isolated worker; the core also refuses credential writes after cancellation.
  for _ = 1, math.min(36, math.floor(pin.expires_in / 5)) do
    xd.sleep(5000)
    local answer = request(session, "/v4/pin/check", { pin = pin.pin, check = pin.check })
    if answer.activated == true then
      local key = h.text(answer.apikey, "API key")
      local title = account_status(h.session(origin, key))
      h.save({ apikey = key })
      return h.status(title)
    end
  end
  error("Approval timed out. Click Connect account to get a new code.")
end
local session = h.session(origin, h.text(h.account().apikey, "saved API key"))
if action == "account" then return h.status(account_status(session)) end
assert(action == "generate", "Unknown plugin action.")
local data = request(session, "/v4/link/unlock", { link = event.url })
if data.delayed and data.delayed ~= 0 then
  local ready = false
  for _ = 1, 36 do
    xd.log("info", "AllDebrid is preparing the file…")
    xd.sleep(5000)
    local pending = request(session, "/v4/link/delayed", { id = data.delayed })
    if pending.status == 2 then data.link = pending.link; ready = true; break end
    assert(pending.status == 1, "AllDebrid could not prepare this file.")
  end
  assert(ready, "The host is still preparing the file. Try again later.")
end
assert(type(data.link) == "string" and #data.link > 0,
  "This link needs a stream-quality choice. Generate a file link on the AllDebrid website.")
return { title = "All Debrid — generated download", single_connection = true,
  items = { h.item(data.link, data.filename, data.filesize, { "debrid.it", "alldebrid.com" }) } }
