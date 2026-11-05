-- Automation: runs when every download in a queue has finished. You choose
-- which queue in the install review, because a package can't name your
-- queues: manifest.json leaves "queue" empty, and the review asks.
--
-- It shows a notification and can start another queue. To start the next
-- queue without a notification, a rule on the Rules tab does that with no code.

-- 1. event.queue is the ID of the queue that finished. Look up its name.
local queue_name = event.queue
for _, queue in ipairs(xd.queues()) do
  if queue.id == event.queue then
    queue_name = queue.name
  end
end

-- 2. Count the finished downloads in that queue.
local finished = 0
for _, download in ipairs(xd.downloads()) do
  if download.queue == event.queue and download.status == "complete" then
    finished = finished + 1
  end
end

-- 3. Show a notification. Scripts can show up to five per run, and each one
--    is also written to the run log.
local text = finished == 1 and "1 download is complete" or finished .. " downloads are complete"
xd.notify("Queue finished: " .. queue_name, text)

-- 4. Read a setting from the Settings tab. Jobs can read settings but never
--    change them. An empty value means: don't start another queue.
local next_queue = xd.setting("next_queue")
if next_queue and next_queue ~= "" then
  xd.start_queue(next_queue) -- A queue's name or its ID both work.
  print("Started the queue " .. next_queue)
end
