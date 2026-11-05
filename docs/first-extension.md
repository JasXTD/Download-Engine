# Make your first extension

This guide walks through one of the examples that come with Download Engine, the Wikimedia Commons reader, line by line. Then you pack it, install it and try it. It takes about fifteen minutes, and you only need Download Engine and a text editor.

An extension is a folder with a `manifest.json` and one or more `.lua` files, packed into one `.de-extension.json` file. [Extensions](extensions.md) has the full reference.

## The examples

The examples are in the `examples\extensions` folder where Download Engine is installed, usually `%LOCALAPPDATA%\Programs\Download Engine`. Each one is a folder you can read and copy, next to its packed file. To install one without packing anything, open **Extensions** and choose **Examples…**.

| Example | Kind | What it shows |
| --- | --- | --- |
| Wikimedia Commons originals | Site reader | One request and one file. This guide. |
| Internet Archive files | Site reader | A list of files, a setting, and leaving out files that can't be downloaded. |
| GitHub release files | Site reader | Two hosts, a request header and a busy API. |
| Sort finished files | Automation, a download finishes | Asking for file access, and how the review asks you. |
| Queue finished | Automation, a queue finishes | A notification, a text setting, and choosing the queue in the review. |

## 1. Copy the example

Work on a copy, so the original stays as it is. In PowerShell:

```powershell
cd "$env:LOCALAPPDATA\Programs\Download Engine"
Copy-Item -Recurse .\examples\extensions\wikimedia-commons "$HOME\my-commons"
```

The folder holds two files, `manifest.json` and `reader.lua`.

## 2. The manifest

```json
{
  "format": "download-engine-extension",
  "format_version": 1,
  "id": "examples.wikimedia-commons",
  "name": "Wikimedia Commons originals",
  "version": "1.0.0",
  "publisher": "Download Engine examples",
  "description": "Gets the full-size original behind a Wikimedia Commons file page, not the preview.",
  "license": "MIT",
  "api_version": 1,
  "permissions": {
    "network": ["commons.wikimedia.org"]
  },
  "jobs": [
    {
      "id": "original-file",
      "name": "Commons file pages",
      "entry": "reader.lua",
      "timeout_seconds": 30,
      "trigger": { "kind": "site_reader", "pages": ["commons.wikimedia.org/wiki/File:*"] }
    }
  ]
}
```

- `format` and `format_version` tell Download Engine this is an extension file it can read.
- `id` finds the installed copy when you install a newer version, so it never changes. Make it yours before you change anything else, for example `yourname.commons`. Ids that start with `examples.` are kept for the examples that come with the app, so a changed copy that keeps one is refused.
- `name`, `publisher` and `description` are what the install review and the Extensions page show. Download Engine doesn't verify the publisher, and the review says so.
- `version` is `MAJOR.MINOR.PATCH`. Each update must be higher than the installed one.
- `api_version` is the Lua API the code was written for. Today that is 1.
- `permissions.network` lists every host the extension may reach. Rust checks each request and each redirect against this list, so `xd.fetch` can't reach anything else. The install review shows the list as "Reads commons.wikimedia.org".
- `jobs` has one job. A job's `id` stays the same across updates, and its `name` appears on the Overview tab.
- `entry` is the Lua file that runs, and `timeout_seconds` stops it if it takes longer.
- `"kind": "site_reader"` makes it a site reader. `pages` says which pages it reads: here, every page whose address starts with `commons.wikimedia.org/wiki/File:`. `*` matches anything. Each page's host must also be in `permissions.network`.

## 3. The reader

A site reader gets the page's address, may read declared hosts, and returns a list of files. Download Engine shows that list, and you choose what to download. The reader can't see or change your downloads.

```lua
local title = event.url:match("^https?://commons%.wikimedia%.org/wiki/([^?#]+)")
assert(title and title:match("^File:"), "Open a Commons file page, such as .../wiki/File:Example.jpg")
```

`event.url` is the page you opened. `match` is Lua's pattern search: `%.` is a literal dot, and `([^?#]+)` captures everything after `/wiki/` up to a `?` or `#`. `assert` stops the run when its first value is false or `nil`. Its message is what you see in the run log, so write it for a person.

```lua
title = title:gsub("%%(%x%x)", function(code)
  return string.char(tonumber(code, 16))
end)
```

Addresses write some characters as codes: "Café" arrives as `Caf%C3%A9`. `gsub` finds every `%` followed by two hexadecimal digits and replaces it with that byte, so `title` becomes `File:Café_at_night.jpg`.

```lua
local api = "https://commons.wikimedia.org/w/api.php?action=query&format=json&formatversion=2"
  .. "&prop=imageinfo&iiprop=url%7Csize%7Cmime&titles=" .. xd.url_encode(title)
local reply = xd.fetch(api)
assert(reply.status == 200, "Commons answered with HTTP " .. reply.status)
```

`..` joins text. The address asks the public MediaWiki API for the file's original address, size and type: `%7C` is `|`, which the API puts between the values it should return. `xd.url_encode` encodes the title as one value, so a `&` in a file name can't break the address.

`xd.fetch` reads the address and returns a table with `status`, the final `url`, `headers` and `body`. An error status such as 404 is returned, not raised, so the next line checks it. A host missing from `permissions.network` would stop the run here.

```lua
local answer = xd.json(reply.body)
local page = answer.query and answer.query.pages and answer.query.pages[1]
assert(page and not page.missing and not page.invalid, "Commons has no file called " .. title)
local original = page.imageinfo and page.imageinfo[1]
assert(original and original.url, "Commons did not return the file's address")
```

`xd.json` turns the reply into Lua tables, and JSON `null` becomes `nil`. Lua lists start at 1, so `pages[1]` is the first page. Writing `a and a.b` returns `nil` instead of stopping with an error when `a` is missing, so an unexpected answer ends in a readable message.

```lua
local filename = page.title:gsub("^File:", "")
return {
  title = page.title,
  items = {
    {
      kind = "file",
      url = original.url,
      filename = filename,
      label = original.mime,
      size = original.size,
    },
  },
}
```

A reader ends with `return`. `title` is shown above the list. Each item needs `kind` and `url`. Use `"file"` for a plain file, and `"hls"` or `"dash"` for a stream. `filename`, `label` and `size` (in bytes, above 0) are optional. A reader can return up to 1,000 items.

## 4. Pack and install

Change the `id` in your copy's `manifest.json` first, then pack the folder:

```powershell
.\de-cli.exe extensions pack "$HOME\my-commons" --out "$HOME\my-commons.de-extension.json"
.\de-cli.exe extensions review "$HOME\my-commons.de-extension.json"
```

`pack` runs the same package checks as an install, and refuses links and any file that isn't `.lua`. `review` shows what installing would do, without saving anything.

In Download Engine, open **Extensions**, choose **Install from file…** and pick your file. The review shows what each job does and what it can reach. Choose **Install**. It installs turned off, so choose **Turn on** when you are ready.

## 5. Try it

Open a Commons file page, such as `https://commons.wikimedia.org/wiki/File:Example.jpg`, with **Add URL** or **Find videos on this page**. The list shows the original file. If something goes wrong, find the run on the **Run history** tab and choose **View log**: it shows your `assert` messages and anything the code printed. Site readers need **Preferences › Connection › Proxy › Direct connection**.

## 6. Change it

- To read another site, change `permissions.network`, the job's `pages` and the code. Keep the list of hosts as short as you can, because the review shows each one.
- Use `print(...)` or `xd.log("warning", ...)` to see values in the run log while you work.
- To ship a change, raise `version`, pack again and install the new file. The review shows what changed, and new hosts, a new publisher or a job that runs at other times need approval.
- Your own scripts on the **Scripts** tab are often simpler for something only you use. An extension is for something you want to share or keep updated.

The other examples are worth reading next. Sort finished files shows what happens when a job asks to change files: the review shows a box to allow it, and you can allow it later on the Overview tab or take it back there. Queue finished leaves its queue empty, so the review asks which of your queues it watches.
