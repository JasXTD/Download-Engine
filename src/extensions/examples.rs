//! The examples that come with Download Engine. Their ids start with `examples.`,
//! and only these exact files may use such an id. Otherwise a file from
//! anywhere could pass itself off as an example, or update one with new code
//! that keeps the file access the person gave the real one.
const PREFIX: &str = "examples.";

/// The packed files from examples/extensions, byte for byte.
pub const FILES: [&[u8]; 5] = [
    include_bytes!("../../examples/extensions/internet-archive.de-extension.json"),
    include_bytes!("../../examples/extensions/wikimedia-commons.de-extension.json"),
    include_bytes!("../../examples/extensions/github-releases.de-extension.json"),
    include_bytes!("../../examples/extensions/sort-by-type.de-extension.json"),
    include_bytes!("../../examples/extensions/queue-finished.de-extension.json"),
];

/// Whether a package with this id must be one of the files above.
pub fn reserved(id: &str) -> bool {
    id.starts_with(PREFIX)
}

/// Whether these bytes are exactly one of the examples that come with the app.
pub fn is_example(bytes: &[u8]) -> bool {
    FILES.contains(&bytes)
}
