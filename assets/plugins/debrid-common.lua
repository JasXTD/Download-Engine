-- Shared by the three separately packaged plugins. No account information is logged.
local M = {}
function M.text(value, label, limit)
  assert(type(value) == "string" and #value > 0 and #value <= (limit or 2048), "Invalid " .. label .. " in service response")
  assert(not value:find("[%c]"), "Invalid " .. label .. " in service response")
  return value
end
function M.session(origin, token)
  local headers = { Accept = "application/json" }
  if token then headers.Authorization = "Bearer " .. M.text(token, "access token") end
  -- Origin-bound sessions refuse cross-origin redirects, including credentials
  -- embedded in POST bodies or device-code query parameters.
  return xd.http_session(origin, { headers = headers })
end
function M.request(session, url, fields)
  local options = { session = session }
  if fields then
    local keys, parts = {}, {}
    for key in pairs(fields) do keys[#keys + 1] = key end
    table.sort(keys)
    for _, key in ipairs(keys) do parts[#parts + 1] = xd.url_encode(key) .. "=" .. xd.url_encode(tostring(fields[key])) end
    options.method = "POST"
    options.body = table.concat(parts, "&")
    options.headers = { ["Content-Type"] = "application/x-www-form-urlencoded" }
  end
  local ok, reply = pcall(xd.fetch, url, options)
  assert(ok, "Cannot reach the service. Check your connection or proxy, then try again.")
  if reply.status == 429 then error("The service is limiting requests. Wait before trying again.") end
  if reply.status >= 500 then error("The service is temporarily unavailable. Try again later.") end
  local parsed, data = pcall(xd.json, reply.body)
  assert(parsed and type(data) == "table", "The service returned an unreadable response.")
  return data, reply.status
end
function M.account()
  local saved = xd.setting("account")
  assert(type(saved) == "string" and #saved > 0, "Connect your account first.")
  local ok, data = pcall(xd.json, saved)
  assert(ok and type(data) == "table", "Saved account is invalid. Connect again.")
  return data
end
function M.save(data)
  xd.save_secret("account", xd.to_json(data))
end
function M.status(title)
  return { title = title, items = {} }
end
function M.item(link, filename, size, domains)
  M.text(link, "download link", 16384)
  local host = link:match("^https?://([^/?#]+)")
  assert(host and not host:find("[@:]"), "The service returned an unsupported download address.")
  host = host:lower()
  local allowed = false
  for _, domain in ipairs(domains) do
    if host == domain or host:sub(-#domain - 1) == "." .. domain then allowed = true end
  end
  assert(allowed, "The service returned an unapproved download host. Report this on GitHub without sharing the private link.")
  if filename ~= nil then M.text(filename, "filename", 1024) end
  if type(size) ~= "number" or size <= 0 or size > 9007199254740991 then size = nil end
  return { kind = "file", url = link, filename = filename, size = size }
end
function M.disconnect()
  xd.save_secret("account", nil)
  return M.status("Disconnected. This app's saved account has been removed.")
end
return M
