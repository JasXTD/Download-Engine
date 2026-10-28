-- Manual trigger. Read-only example: no downloads or files are changed.
assert(xd.api_version == 1, "This example requires Lua API 1")
print("Download Engine", xd.app_version, "Lua API", xd.api_version)
print("Role:", xd.script_kind, "Can fetch pages:", xd.supports("xd.fetch"))
for _, queue in ipairs(xd.queues()) do
    print("Queue:", queue.name, "Running:", queue.running)
end
local counts = {}
for _, download in ipairs(xd.downloads()) do
    counts[download.status] = (counts[download.status] or 0) + 1
end
print("Download counts:", xd.to_json(counts))
