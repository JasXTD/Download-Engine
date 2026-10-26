-- Convenience functions over the app's existing CLI command contract.
function xd.downloads() return xd.command("list") end
function xd.status(id) return xd.command("status", id) end
function xd.queues() return xd.command("queues") end
function xd.create_queue(name) return xd.command("queue", "create", name) end
function xd.start_queue(queue) return xd.command("queue", "start", queue) end
function xd.pause_queue(queue) return xd.command("queue", "pause", queue) end
function xd.pause(id) return xd.command("pause", id) end
function xd.resume(id) return xd.command("resume", id) end
function xd.pause_all() return xd.command("pause-all") end
function xd.add(url, options)
    local args = {"add", url}
    options = options or {}
    for key in pairs(options) do
        assert(key == "queue" or key == "folder" or key == "name", "Unknown xd.add option: " .. tostring(key))
    end
    for _, key in ipairs({"queue", "folder", "name"}) do
        if options[key] ~= nil then
            assert(type(options[key]) == "string", "xd.add " .. key .. " must be a string")
            args[#args + 1] = "--" .. key
            args[#args + 1] = options[key]
        end
    end
    return xd.command(table.unpack(args))
end
function xd.retry(id) return xd.command("retry", id) end
function xd.restart(id) return xd.command("restart", id) end
function xd.rename(id, name) return xd.command("rename", id, name) end
-- A move to another drive copies the file, so it may take longer than other commands.
function xd.move(id, folder) return xd.command("move", id, folder, "--wait", "120") end
function xd.remove(id, options)
    options = options or {}
    for key in pairs(options) do
        assert(key == "delete_file", "Unknown xd.remove option: " .. tostring(key))
    end
    if options.delete_file then return xd.command("remove", id, "--delete-file") end
    return xd.command("remove", id)
end
function xd.notify(title, text)
    assert(type(title) == "string", "xd.notify title must be a string")
    assert(text == nil or type(text) == "string", "xd.notify text must be a string")
    return xd.command("notify", title, text or "")
end
