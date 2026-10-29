//! Embedded Lua used only in a scheduler-owned CLI worker, never on the UI thread.
//! Automation scripts can see and change downloads but cannot reach the web;
//! site readers can read the web but cannot see or change downloads.
use super::fetch::{FetchReply, FetchRequest};
use anyhow::{Result, ensure};
use mlua::{
    Lua, LuaSerdeExt, StdLib, Table, Value, Variadic, chunk::ChunkMode, serde::SerializeOptions,
};
use std::{io::Write, time::Duration};

const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const API: &str = include_str!("api.lua");
/// The automation functions a site reader does not get.
const AUTOMATION_ONLY: [&str; 17] = [
    "command",
    "downloads",
    "status",
    "add",
    "queues",
    "create_queue",
    "start_queue",
    "pause_queue",
    "pause",
    "resume",
    "pause_all",
    "retry",
    "restart",
    "rename",
    "move",
    "remove",
    "notify",
];
/// xd.store's functions; a site reader gets none of them either.
const STORE: [&str; 3] = ["get", "set", "keys"];
const READER_REFUSAL: &str =
    "Site readers cannot see or change your downloads; use another script for that";

pub(crate) fn to_lua(lua: &Lua, value: &impl serde::Serialize) -> mlua::Result<Value> {
    lua.to_value_with(
        value,
        SerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false),
    )
}

/// A fresh, memory-capped Lua with no file, OS or module access, plus the
/// `event`, `print` and shared `xd` helpers both kinds of script get.
fn sandbox(source: &str, event: &serde_json::Value) -> Result<(Lua, Table)> {
    ensure!(
        source.len() <= super::MAX_SCRIPT_BYTES,
        "Script exceeds 64 KiB"
    );
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE,
        Default::default(),
    )?;
    lua.set_memory_limit(MEMORY_LIMIT)?;
    let globals = lua.globals();
    // Base Lua includes file loaders even without the io/package libraries.
    // Only text entered into the editor is loaded; native modules and bytecode
    // are not part of this application scripting API.
    for name in ["dofile", "loadfile", "load"] {
        globals.set(name, Value::Nil)?;
    }
    globals.set("event", to_lua(&lua, event)?)?;
    let xd = lua.create_table()?;
    xd.set(
        "sleep",
        lua.create_function(|_, millis: u64| {
            if millis > 60_000 {
                return Err(mlua::Error::runtime(
                    "xd.sleep accepts 0..60000 milliseconds",
                ));
            }
            std::thread::sleep(Duration::from_millis(millis));
            Ok(())
        })?,
    )?;
    xd.set(
        "log",
        lua.create_function(|_, (level, message): (String, String)| {
            match level.as_str() {
                "info" => writeln!(std::io::stdout().lock(), "{message}"),
                "warn" | "warning" => {
                    let mut stdout = std::io::stdout().lock();
                    for line in message.split('\n') {
                        writeln!(stdout, "WARNING: {line}").map_err(mlua::Error::external)?;
                    }
                    Ok(())
                }
                "error" => writeln!(std::io::stderr().lock(), "ERROR: {message}"),
                _ => {
                    return Err(mlua::Error::runtime(
                        "Log level must be info, warning or error",
                    ));
                }
            }
            .map_err(mlua::Error::external)
        })?,
    )?;
    globals.set(
        "print",
        lua.create_function(|_, args: Variadic<Value>| {
            // Stream each value instead of concatenating repeated large strings.
            let mut stdout = std::io::stdout().lock();
            for (i, value) in args.iter().enumerate() {
                if i > 0 {
                    stdout.write_all(b"\t").map_err(mlua::Error::external)?;
                }
                stdout
                    .write_all(value.to_string()?.as_bytes())
                    .map_err(mlua::Error::external)?;
            }
            stdout.write_all(b"\n").map_err(mlua::Error::external)
        })?,
    )?;
    Ok((lua, xd))
}

/// The callback uses the same CLI command/receipt path as PowerShell. Its caller
/// must run in a guarded worker process: the scheduler owns Stop and timeouts.
pub fn execute_lua(
    source: &str,
    event: &serde_json::Value,
    command: impl Fn(Vec<String>) -> Result<serde_json::Value> + 'static,
) -> Result<()> {
    let (lua, xd) = sandbox(source, event)?;
    super::helpers::install(&lua, &xd, false)?;
    let command = std::rc::Rc::new(command);
    let call = command.clone();
    xd.set(
        "command",
        lua.create_function(move |lua, values: Variadic<Value>| {
            if values.is_empty() || values.len() > 128 {
                return Err(mlua::Error::runtime(
                    "xd.command expects 1..128 string arguments, at most 64 KiB",
                ));
            }
            // Check borrowed Lua strings before copying. Repeated references to
            // one large Lua string must not cause unbounded Rust allocations.
            let mut args = Vec::with_capacity(values.len());
            let mut bytes = 0;
            for value in values {
                let Value::String(value) = value else {
                    return Err(mlua::Error::runtime("xd.command arguments must be strings"));
                };
                let value = value.to_str()?;
                bytes += value.len();
                if bytes > 65_536 {
                    return Err(mlua::Error::runtime("xd.command arguments exceed 64 KiB"));
                }
                args.push(value.to_owned());
            }
            if !matches!(
                args[0].as_str(),
                "list"
                    | "status"
                    | "add"
                    | "pause"
                    | "resume"
                    | "pause-all"
                    | "queues"
                    | "queue"
                    | "request"
                    | "cancel"
                    | "automation"
                    | "retry"
                    | "restart"
                    | "rename"
                    | "move"
                    | "remove"
                    | "notify"
                    | "state"
            ) {
                return Err(mlua::Error::runtime(
                    "This command is not available in Lua; see the script guide",
                ));
            }
            let result = call(args).map_err(mlua::Error::external)?;
            to_lua(lua, &result)
        })?,
    )?;
    // What the script keeps between runs. Values are any JSON-like Lua value;
    // setting nil removes the key.
    let store = lua.create_table()?;
    let call = command.clone();
    store.set(
        "get",
        lua.create_function(move |lua, key: String| {
            let value =
                call(vec!["state".into(), "get".into(), key]).map_err(mlua::Error::external)?;
            to_lua(lua, &value)
        })?,
    )?;
    let call = command.clone();
    store.set(
        "set",
        lua.create_function(move |lua, (key, value): (String, Value)| {
            let value = super::json::from_lua(lua, value, super::MAX_STATE_VALUE_BYTES)?;
            let text = serde_json::to_string(&value).map_err(mlua::Error::external)?;
            if text.len() > super::MAX_STATE_VALUE_BYTES {
                return Err(mlua::Error::runtime("A stored value can be at most 16 KiB"));
            }
            call(vec!["state".into(), "set".into(), key, text]).map_err(mlua::Error::external)?;
            Ok(())
        })?,
    )?;
    store.set(
        "keys",
        lua.create_function(move |lua, ()| {
            let keys =
                command(vec!["state".into(), "keys".into()]).map_err(mlua::Error::external)?;
            to_lua(lua, &keys)
        })?,
    )?;
    xd.set("store", store)?;
    xd.set(
        "fetch",
        lua.create_function(|_, _: Variadic<Value>| -> mlua::Result<()> {
            Err(mlua::Error::runtime(
                "xd.fetch is only available in site readers",
            ))
        })?,
    )?;
    lua.globals().set("xd", xd)?;
    lua.load(API)
        .set_name("xd API")
        .set_mode(ChunkMode::Text)
        .exec()?;
    lua.load(source)
        .set_name("automation.lua")
        .set_mode(ChunkMode::Text)
        .exec()?;
    Ok(())
}

/// Run a site reader and return the value its script returns, as JSON.
/// `event.url` is the page being read.
pub fn execute_reader(
    source: &str,
    event: &serde_json::Value,
    fetch: impl FnMut(FetchRequest) -> Result<FetchReply> + 'static,
) -> Result<serde_json::Value> {
    let (lua, xd) = sandbox(source, event)?;
    super::helpers::install(&lua, &xd, true)?;
    let refuse = || {
        lua.create_function(|_, _: Variadic<Value>| -> mlua::Result<()> {
            Err(mlua::Error::runtime(READER_REFUSAL))
        })
    };
    for name in AUTOMATION_ONLY {
        xd.set(name, refuse()?)?;
    }
    let store = lua.create_table()?;
    for name in STORE {
        store.set(name, refuse()?)?;
    }
    xd.set("store", store)?;
    install_fetch(&lua, &xd, fetch)?;
    lua.globals().set("xd", xd)?;
    let value: Value = lua
        .load(source)
        .set_name("reader.lua")
        .set_mode(ChunkMode::Text)
        .eval()?;
    Ok(super::json::from_lua(
        &lua,
        value,
        super::reading::MAX_READING_BYTES,
    )?)
}

pub(crate) fn install_fetch(
    lua: &Lua,
    xd: &Table,
    mut fetch: impl FnMut(FetchRequest) -> Result<FetchReply> + 'static,
) -> Result<()> {
    xd.set(
        "fetch",
        lua.create_function_mut(move |lua, (url, options): (String, Option<Table>)| {
            let mut request = FetchRequest {
                url,
                ..Default::default()
            };
            if let Some(options) = options {
                for pair in options.pairs::<String, Value>() {
                    let (key, value) = pair?;
                    match (key.as_str(), value) {
                        ("method", Value::String(v)) => {
                            request.method = Some(v.to_str()?.to_owned())
                        }
                        ("body", Value::String(v)) => request.body = Some(v.to_str()?.to_owned()),
                        ("headers", Value::Table(headers)) => {
                            for header in headers.pairs::<String, String>() {
                                request.headers.push(header?);
                            }
                        }
                        ("method" | "body" | "headers", _) => {
                            return Err(mlua::Error::runtime(format!(
                                "xd.fetch option {key} has the wrong type"
                            )));
                        }
                        _ => {
                            return Err(mlua::Error::runtime(format!(
                                "Unknown xd.fetch option: {key}"
                            )));
                        }
                    }
                }
            }
            let reply = fetch(request).map_err(mlua::Error::external)?;
            to_lua(lua, &reply)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn native_conversion_rejects_expansion_depth_and_cycles_before_allocating_results() {
        for source in [
            "local s=string.rep('x',1024*1024); local t={}; for i=1,80 do t[i]=s end; return t",
            "local t={}; local r=t; for i=1,100 do t.child={}; t=t.child end; return r",
            "local t={}; t.self=t; return t",
        ] {
            assert!(execute_reader(source, &json!({}), |_| unreachable!()).is_err());
        }
        execute_lua(
            r#"
            local s=string.rep('x', 16384)
            assert(not pcall(xd.store.set, 'too-large', {s,s}))
            local t={}; t.self=t
            assert(not pcall(xd.to_json, t))
            assert(not pcall(xd.to_json, string.char(255)))
            assert(not pcall(xd.json, string.rep(' ', 8*1024*1024+1)))
            local many={}; for i=1,65536 do many[i]=true end
            assert(not pcall(xd.to_json, many))
            assert(not pcall(xd.to_json, {string.rep('x', 5*1024*1024), string.rep('y', 5*1024*1024)}))
            local shared={name='ok'}
            assert(xd.to_json({shared,shared}) == '[{"name":"ok"},{"name":"ok"}]')
        "#,
            &json!({}),
            |_| panic!("Oversized stored data must not reach the command API"),
        )
        .unwrap();
    }
    #[test]
    fn versioned_core_helpers_work_in_both_roles_and_report_the_real_contract() {
        let common = r#"
            assert(xd.api_version == 1 and type(xd.app_version) == 'string')
            assert(xd.supports('xd.html_select') and xd.supports('xd.json'))
            assert(not xd.supports('xd.execute'))
            local data = xd.json(xd.to_json({name='Lua'}))
            assert(data.name == 'Lua')
            local links = xd.html_select('<a href="../f.zip?a=1&amp;b=2">File</a>', 'a[href]', 'href')
            assert(xd.url_join('https://example.com/watch/1', links[1]) == 'https://example.com/f.zip?a=1&b=2')
            assert(not pcall(xd.url_join, 'https://example.com', 'javascript:alert(1)'))
            assert(not pcall(xd.url_join, 'https://example.com', 'https://user:secret@example.com'))
            assert(os == nil and io == nil and package == nil and loadfile == nil)
        "#;
        for reader in [false, true] {
            let mut source = common.to_owned();
            source.push_str(&format!(
                "\nassert(xd.script_kind == '{}')\n",
                if reader { "site_reader" } else { "automation" }
            ));
            for function in super::super::api::FUNCTIONS {
                source.push_str(&format!(
                    "assert(xd.supports('{}') == {})\n",
                    function.name,
                    function.available(reader)
                ));
                if function.available(reader) {
                    source.push_str(&format!("assert(type({}) == 'function')\n", function.name));
                }
            }
            if reader {
                source.push_str(
                    "assert(not pcall(xd.add, 'https://example.com/file')); return {items={}}\n",
                );
                execute_reader(&source, &json!({}), |_| panic!("No network needed")).unwrap();
            } else {
                source.push_str("assert(not pcall(xd.fetch, 'https://example.com'))\n");
                execute_lua(&source, &json!({}), |_| panic!("No command needed")).unwrap();
            }
        }
    }

    #[test]
    fn custom_reader_composes_host_fetch_html_and_url_helpers_into_valid_downloads() {
        let result = execute_reader(r#"
            local page = xd.fetch(event.url)
            assert(page.status == 200)
            local items = {}
            for _, href in ipairs(xd.html_select(page.body, 'a.download', 'href')) do
                items[#items+1] = {kind='file', url=xd.url_join(page.url, href)}
            end
            return {title=xd.html_select(page.body, 'title')[1], items=items}
        "#, &json!({"url":"https://example.com/files/"}), |request| {
            assert_eq!(request.url, "https://example.com/files/");
            Ok(FetchReply { status:200, url:request.url,
                headers: Default::default(),
                body:"<title>My files</title><a class='download' href='../one.zip'>One</a><a class='download' href='two.pdf'>Two</a>".into() })
        }).unwrap();
        let reading = super::super::Reading::from_value(result).unwrap();
        assert_eq!(reading.title.as_deref(), Some("My files"));
        assert_eq!(reading.items.len(), 2);
        assert_eq!(reading.items[0].url, "https://example.com/one.zip");
        assert_eq!(reading.items[1].url, "https://example.com/files/two.pdf");
    }
    #[test]
    fn lua_supports_functions_loops_events_and_app_commands() {
        execute_lua(r#"
            assert(_VERSION == "Lua 5.4")
            local function double(n) return n * 2 end
            local sum = 0
            for _, item in ipairs(event.downloads) do sum = sum + double(item.size) end
            assert(sum == 12 and event.queue == nil)
            assert(xd.queues()[1].name == "main")
            assert(xd.add("https://example.test/file", {queue="Later", name="my file.zip"})[1].id == "job")
            assert(xd.status("job").status == "complete")
        "#, &json!({"queue":null,"downloads":[{"size":2},{"size":4}]}), |args| {
            Ok(match args[0].as_str() {
                "queues" => json!([{"name":"main"}]),
                "add" => { assert_eq!(args, ["add","https://example.test/file","--queue","Later","--name","my file.zip"]); json!([{"id":"job"}]) },
                "status" => json!({"status":"complete"}),
                _ => panic!("Unexpected command"),
            })
        }).unwrap();
    }
    #[test]
    fn lua_failures_and_limits_are_explicit() {
        for code in [
            "local =",
            "error('expected')",
            "xd.sleep(-1)",
            "xd.sleep(60001)",
            "xd.log('unknown','x')",
            "xd.add('url',{typo=true})",
            "xd.command('batch','file')",
            "xd.command('mcp-approve-script','id','1')",
            "xd.command('mcp-revoke-script','id')",
            "xd.command('scripts','run','id')",
            "xd.command()",
            "xd.command('status', 123)",
            "xd.command('status', string.rep('a', 65536))",
            "local a={}; for i=1,129 do a[i]='queues' end; xd.command(table.unpack(a))",
            "local t = {}; for i=1,100 do t[i]=string.rep('x', 1000000) end",
        ] {
            assert!(
                execute_lua(code, &json!({}), |_| Ok(json!(null))).is_err(),
                "{code}"
            );
        }
        let error = execute_lua("xd.queues()", &json!({}), |_| {
            anyhow::bail!("CLI rejected command")
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("CLI rejected command"));
    }
    #[test]
    fn file_actions_notify_and_store_use_the_cli_contract() {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = seen.clone();
        execute_lua(
            r#"
            xd.retry("a")
            xd.restart("a")
            xd.rename("a", "Holiday photos.zip")
            xd.move("a", [[C:\Downloads\Archives]])
            xd.remove("a")
            xd.remove("a", {delete_file = true})
            xd.notify("2 downloads retried")
            xd.notify("Done", "Ünïcode text")
            xd.store.set("seen", {hosts = {"a.example"}, count = 2})
            xd.store.set("gone", nil)
            assert(xd.store.get("seen").count == 2)
            assert(xd.store.get("missing") == nil)
            assert(xd.store.keys()[1] == "seen")
            "#,
            &json!({}),
            move |args| {
                log.borrow_mut().push(args.clone());
                Ok(match (args[0].as_str(), args.get(1).map(String::as_str)) {
                    ("state", Some("get")) if args[2] == "seen" => json!({"count": 2}),
                    ("state", Some("keys")) => json!(["seen"]),
                    _ => json!(null),
                })
            },
        )
        .unwrap();
        let seen = seen.borrow();
        let expected: Vec<Vec<&str>> = vec![
            vec!["retry", "a"],
            vec!["restart", "a"],
            vec!["rename", "a", "Holiday photos.zip"],
            vec!["move", "a", r"C:\Downloads\Archives", "--wait", "120"],
            vec!["remove", "a"],
            vec!["remove", "a", "--delete-file"],
            vec!["notify", "2 downloads retried", ""],
            vec!["notify", "Done", "Ünïcode text"],
        ];
        for (args, expected) in seen.iter().zip(&expected) {
            assert_eq!(args, expected);
        }
        let stored = &seen[expected.len()];
        assert_eq!(stored[..3], ["state", "set", "seen"]);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stored[3]).unwrap(),
            json!({"hosts": ["a.example"], "count": 2})
        );
        assert_eq!(seen[expected.len() + 1], ["state", "set", "gone", "null"]);
        assert_eq!(seen[expected.len() + 2], ["state", "get", "seen"]);
        for code in [
            "xd.remove('a', {delete = true})",
            "xd.notify(1)",
            "xd.notify('title', {})",
            "xd.store.set('f', print)",
            "xd.store.set('big', string.rep('x', 16385))",
        ] {
            assert!(
                execute_lua(code, &json!({}), |_| Ok(json!(null))).is_err(),
                "{code}"
            );
        }
    }

    #[test]
    fn documented_retry_and_sorting_examples_send_the_expected_commands() {
        let document = include_str!("../../docs/automation.md").replace("\r\n", "\n");
        let example = |marker: &str| {
            document
                .split("```lua\n")
                .find(|block| block.starts_with(marker))
                .and_then(|block| block.split("```").next())
                .unwrap_or_else(|| panic!("Missing example: {marker}"))
                .to_owned()
        };
        let run = |source: &str, event: serde_json::Value, refuse: bool| {
            let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let log = seen.clone();
            execute_lua(source, &event, move |args| {
                log.borrow_mut().push(args.clone());
                if refuse && args[0] == "retry" {
                    anyhow::bail!("Scripts already retried this download 5 times");
                }
                Ok(json!(null))
            })
            .unwrap();
            seen.take()
        };
        let retry = example("-- Download failed:");
        let failed = json!({"downloads": [{"id": "a", "name": "a.zip", "error": "timed out"}]});
        assert_eq!(run(&retry, failed.clone(), false), [["retry", "a"]]);
        assert_eq!(
            run(&retry, failed, true),
            [
                vec!["retry", "a"],
                vec!["notify", "Download still failing", "a.zip: timed out"]
            ]
        );
        let sort = example("-- Download completed:");
        let finished = json!({"downloads": [
            {"id": "p", "name": "Paper.PDF", "saved_path": r"C:\Downloads\Documents\Paper.PDF"},
            {"id": "z", "name": "data.zip", "saved_path": r"C:\Downloads\Compressed\data.zip"},
        ]});
        assert_eq!(
            run(&sort, finished, false),
            [[
                "move",
                "p",
                r"C:\Downloads\Documents\Reports",
                "--wait",
                "120"
            ]]
        );
    }
    #[test]
    fn lua_exposes_no_shell_file_or_native_module_loaders() {
        execute_lua("assert(io == nil and os == nil and package == nil and require == nil and debug == nil and loadfile == nil and dofile == nil and load == nil)", &json!({}), |_| unreachable!()).unwrap();
        assert!(execute_lua("\x1bLua", &json!({}), |_| unreachable!()).is_err());
    }

    #[test]
    fn automation_scripts_cannot_fetch_and_readers_cannot_touch_downloads() {
        let error = execute_lua(
            "xd.fetch('https://example.com/')",
            &json!({}),
            |_| unreachable!(),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("only available in site readers"));
        for name in AUTOMATION_ONLY
            .iter()
            .map(|name| name.to_string())
            .chain(STORE.iter().map(|name| format!("store.{name}")))
        {
            let error = execute_reader(
                &format!("return xd.{name}('x')"),
                &json!({"url": "https://example.com/watch/1"}),
                |_| unreachable!(),
            )
            .unwrap_err();
            assert!(
                format!("{error:#}").contains("cannot see or change your downloads"),
                "{name}"
            );
        }
        execute_reader(
            "assert(io == nil and os == nil and require == nil and load == nil); return {items = {}}",
            &json!({}),
            |_| unreachable!(),
        )
        .unwrap();
    }

    #[test]
    fn readers_fetch_parse_and_return_what_they_found() {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = seen.clone();
        let value = execute_reader(
            r#"
            local id = event.url:match("/watch/(%w+)")
            local reply = xd.fetch("https://api.example.com/v/" .. xd.url_encode(id .. " x&y"),
                { method = "POST", headers = { ["Content-Type"] = "application/json" }, body = xd.to_json({ id = id }) })
            assert(reply.status == 200 and reply.headers["content-type"] == "application/json")
            local video = xd.json(reply.body)
            assert(video.missing == nil)
            return { title = video.title, items = { { kind = "hls", url = video.hls, label = "All" } } }
            "#,
            &json!({"url": "https://example.com/watch/a1"}),
            move |request| {
                log.borrow_mut().push(request.clone());
                Ok(FetchReply {
                    status: 200,
                    url: request.url,
                    headers: [("content-type".to_string(), "application/json".to_string())].into(),
                    body: r#"{"title":"Nature","hls":"https://cdn.example.com/m.m3u8","missing":null}"#.into(),
                })
            },
        )
        .unwrap();
        assert_eq!(
            value,
            json!({"title": "Nature", "items": [{"kind": "hls", "url": "https://cdn.example.com/m.m3u8", "label": "All"}]})
        );
        let request = &seen.borrow()[0];
        assert_eq!(request.url, "https://api.example.com/v/a1%20x%26y");
        assert_eq!(request.method.as_deref(), Some("POST"));
        assert_eq!(
            request.headers,
            [("Content-Type".to_string(), "application/json".to_string())]
        );
        assert_eq!(request.body.as_deref(), Some(r#"{"id":"a1"}"#));
        for code in [
            "xd.fetch('https://example.com/', { timeout = 5 })",
            "xd.fetch('https://example.com/', { method = 1 })",
            "xd.json('{not json')",
            "error('Not a video page')",
        ] {
            assert!(
                execute_reader(code, &json!({}), |_| anyhow::bail!("no network")).is_err(),
                "{code}"
            );
        }
        let failed = execute_reader(
            "return xd.fetch('https://example.com/')",
            &json!({}),
            |_| {
                anyhow::bail!(
                    "xd.fetch refused 192.168.1.1: site readers can only reach the public internet"
                )
            },
        )
        .unwrap_err();
        assert!(format!("{failed:#}").contains("public internet"));
        assert_eq!(
            execute_reader("return nil", &json!({}), |_| unreachable!()).unwrap(),
            json!(null)
        );
    }
}
