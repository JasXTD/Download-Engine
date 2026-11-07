-- Automation: when downloads finish, move each file into a folder named after
-- its type, next to where it was saved:
--   Downloads\holiday.mp4  ->  Downloads\Videos\holiday.mp4
--
-- Moving files needs file access. manifest.json asks for it with
-- "change_files": true, and the install review shows a box to allow it. If
-- you leave the box clear, each move fails and the run log says why.
--
-- Download Engine's category folders choose a folder when a download is
-- added. This example tidies up after a download finishes instead, so use
-- one or the other.

-- File types, by the last part of the file name. To change them, open the
-- Code tab, choose Copy to my scripts and edit your copy.
local types = {
  Videos = { "mp4", "mkv", "webm", "avi", "mov" },
  Music = { "mp3", "flac", "wav", "m4a", "ogg", "opus" },
  Pictures = { "jpg", "jpeg", "png", "gif", "webp" },
  Documents = { "pdf", "docx", "xlsx", "pptx", "txt", "epub" },
  Archives = { "zip", "7z", "rar", "tar", "gz" },
  Programs = { "exe", "msi", "msix", "apk", "dmg" },
}

-- Turn that list around, so folder_for["mp4"] is "Videos".
local folder_for = {}
for folder, endings in pairs(types) do
  for _, ending in ipairs(endings) do
    folder_for[ending] = folder
  end
end

-- Works out where one finished file should go, or returns nil to leave it.
local function target_for(path)
  local ending = path:match("%.(%w+)$")
  local folder = ending and folder_for[ending:lower()]
  if not folder then
    return nil -- Not one of the types above.
  end
  -- Split "C:\Downloads\holiday.mp4" into "C:\Downloads" and "\".
  -- macOS and Linux use "/" between folders, so both are accepted.
  local parent, separator = path:match("^(.*)([/\\])[^/\\]+$")
  if not parent then
    return nil
  end
  -- Leave files that are already in a folder with that name.
  local current = parent:match("[^/\\]+$") or ""
  if current:lower() == folder:lower() then
    return nil
  end
  return parent .. separator .. folder
end

-- event.downloads holds the downloads that just finished, up to 100 at once.
for _, download in ipairs(event.downloads) do
  -- saved_path is empty when the download was removed in the meantime.
  local target = download.saved_path and target_for(download.saved_path)
  if target then
    local name = download.name or download.id
    -- pcall catches an error, so one file that can't move doesn't stop the rest.
    local moved, problem = pcall(xd.move, download.id, target)
    if moved then
      print("Moved " .. name .. " to " .. target)
    else
      -- The first line says why. The lines after it say where in the code.
      local reason = tostring(problem):match("[^\n]+")
      xd.log("warning", "Could not move " .. name .. ": " .. reason)
    end
  end
end
