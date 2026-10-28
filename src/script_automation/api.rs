//! Versioned host API shared by the Lua runtime, editor completion and reference.
pub const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Shared,
    Automation,
    Reader,
}

pub struct Function {
    pub name: &'static str,
    pub arguments: &'static str,
    pub description: &'static str,
    pub access: Access,
}
impl Function {
    pub fn available(&self, reader: bool) -> bool {
        self.access == Access::Shared
            || self.access
                == if reader {
                    Access::Reader
                } else {
                    Access::Automation
                }
    }
}

macro_rules! functions {
    ($(($name:literal, $arguments:literal, $description:literal, $access:ident)),* $(,)?) => {
        pub const FUNCTIONS: &[Function] = &[$(Function {
            name: $name, arguments: $arguments, description: $description, access: Access::$access,
        }),*];
    };
}
functions![
    (
        "xd.supports",
        "name",
        "Check whether a function is available for this trigger; file permissions still apply.",
        Shared
    ),
    (
        "xd.log",
        "level, message",
        "Write an info, warning or error message to this run's log.",
        Shared
    ),
    (
        "xd.sleep",
        "milliseconds",
        "Wait up to 60,000 milliseconds; Stop and the run timeout remain active.",
        Shared
    ),
    ("xd.json", "text", "Parse JSON into Lua tables.", Shared),
    ("xd.to_json", "value", "Encode a Lua value as JSON.", Shared),
    ("xd.url_encode", "text", "Encode one URL component.", Shared),
    (
        "xd.url_join",
        "base, relative",
        "Resolve a relative HTTP(S) link; this makes no network request.",
        Shared
    ),
    (
        "xd.html_select",
        "html, selector, attribute",
        "Select HTML text, or an optional attribute, using CSS. Returns a list of strings; does not run JavaScript.",
        Shared
    ),
    (
        "xd.fetch",
        "url, options",
        "Read a public page/API. Options: method, headers, body. Direct connection only.",
        Reader
    ),
    (
        "xd.downloads",
        "",
        "List downloads with their current status.",
        Automation
    ),
    ("xd.status", "id", "Read one download's status.", Automation),
    (
        "xd.add",
        "url, options",
        "Add a download. Options: queue, folder, name.",
        Automation
    ),
    ("xd.queues", "", "List queues.", Automation),
    (
        "xd.create_queue",
        "name",
        "Create a queue, initially paused.",
        Automation
    ),
    (
        "xd.start_queue",
        "queue",
        "Start a queue by ID or name.",
        Automation
    ),
    (
        "xd.pause_queue",
        "queue",
        "Pause a queue by ID or name.",
        Automation
    ),
    (
        "xd.pause",
        "id",
        "Pause a download, keeping saved progress.",
        Automation
    ),
    ("xd.resume", "id", "Resume a paused download.", Automation),
    ("xd.pause_all", "", "Pause all downloads.", Automation),
    (
        "xd.retry",
        "id",
        "Retry a failed download, retaining reusable progress.",
        Automation
    ),
    (
        "xd.restart",
        "id",
        "Restart a download from the beginning, discarding partial progress. Retry limits apply.",
        Automation
    ),
    (
        "xd.rename",
        "id, name",
        "Rename a download's file; file permission is required.",
        Automation
    ),
    (
        "xd.move",
        "id, folder",
        "Move a completed file within download folders; file permission is required.",
        Automation
    ),
    (
        "xd.remove",
        "id, options",
        "Remove a download; file permission is required. delete_file also removes the saved file.",
        Automation
    ),
    (
        "xd.notify",
        "title, text",
        "Show a notification, limited to five per run.",
        Automation
    ),
    (
        "xd.store.get",
        "key",
        "Read a value saved by this script.",
        Automation
    ),
    (
        "xd.store.set",
        "key, value",
        "Save a JSON-compatible value, or nil to remove it.",
        Automation
    ),
    (
        "xd.store.keys",
        "",
        "List this script's stored keys.",
        Automation
    ),
    (
        "xd.command",
        "...",
        "Call the supported CLI command contract. Prefer named functions above.",
        Automation
    ),
];

pub const PROPERTIES: &[&str] = &["xd.api_version", "xd.app_version", "xd.script_kind"];

pub fn supports(name: &str, reader: bool) -> bool {
    FUNCTIONS
        .iter()
        .any(|f| f.name == name && f.available(reader))
}
