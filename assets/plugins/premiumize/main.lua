local h = require("common")
local origin = "https://www.premiumize.me"
local errors = {
  authentication_failed = "The API key is missing, invalid or expired. Save a new key in this plugin's Settings.",
  permission_denied = "Premiumize denied this request. Check your account on its website.",
  service_unsupported = "Premiumize does not support this source.",
  not_found = "This file is no longer available.",
  service_down = "This host is temporarily unavailable.",
  service_limit_reached = "Your account's allowance for this host has been used.",
  account_limit_reached = "Your Premiumize fair-use allowance has been used.",
  rate_limit_reached = "The service is limiting requests. Try again later.",
  link_generation_failed = "Premiumize could not generate this link yet. Try again later.",
}
if xd.extension.job == "disconnect" then return h.disconnect() end
local key = xd.setting("account")
assert(type(key) == "string" and #key > 0,
  "Open the service website, copy your API key, then save it in this plugin's Settings.")
local session = h.session(origin, key)
local function request(path, fields)
  local data, status = h.request(session, origin .. "/api/" .. path, fields)
  assert(status < 400 and data.status == "success",
    errors[data.code] or "Premiumize refused this request. Check your account and source availability on its website.")
  return data
end
if xd.extension.job == "connect" or xd.extension.job == "account" then
  local account = request("account/info")
  assert(type(account.customer_id) == "string" or type(account.customer_id) == "number",
    "Premiumize returned incomplete account details.")
  -- Do not infer that an account is currently premium from an expiry timestamp
  -- alone, or persist identifiers and personal account data.
  local title = "Premiumize.me connected."
  if type(account.limit_used) == "number" and account.limit_used >= 0 and account.limit_used <= 1 then
    title = title .. " Fair-use allowance used: " .. math.floor(account.limit_used * 100) .. "%."
  end
  return h.status(title)
end
assert(xd.extension.job == "generate", "Unknown plugin action.")
local response = request("transfer/directdl", { src = event.url })
assert(type(response.content) == "table", "Premiumize did not return a file list.")
local items, names = {}, {}
for _, file in ipairs(response.content) do
  assert(#items < 1000, "The service returned too many links.")
  local path = h.text(file.path, "file path", 1024)
  local name = path:match("[^/\\]+$")
  assert(name and name ~= "." and name ~= "..", "Invalid generated filename.")
  -- All files go through the ordinary destination review, never service paths.
  local original, number = name, 2
  while names[name:lower()] do
    local stem, suffix = original:match("^(.*)(%.[^.]*)$")
    name = (stem or original) .. " (" .. number .. ")" .. (suffix or "")
    number = number + 1
  end
  names[name:lower()] = true
  items[#items + 1] = h.item(file.link, name, file.size, { "premiumize.me", "energycdn.com" })
end
assert(#items > 0, "Premiumize did not return a downloadable file. The source may not be ready for direct download.")
return { title = "Premiumize.me — generated downloads", single_connection = true, items = items }
