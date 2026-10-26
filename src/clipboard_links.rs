//! Optional clipboard link offers. The Windows listener exists only while the
//! setting is on, reads Unicode text only, and never logs or stores the text.
use std::hash::{BuildHasher, RandomState};

/// Clipboard text longer than this is ignored without being read.
pub const MAX_TEXT_UNITS: usize = 8 * 1024;

/// Final path extensions that are offered. Web pages, plain text, source and
/// script files, images and playlists are left out: their links usually open
/// in a browser, and copying them should not prompt for a download.
const FILE_EXTENSIONS: [&[&str]; 5] = [ARCHIVES, PACKAGES, VIDEO, AUDIO, DOCUMENTS];
const ARCHIVES: &[&str] = &[
    "7z", "bz2", "cab", "gz", "img", "iso", "rar", "tar", "tbz2", "tgz", "txz", "xz", "zip", "zst",
];
const PACKAGES: &[&str] = &[
    "apk",
    "appimage",
    "appx",
    "appxbundle",
    "deb",
    "dmg",
    "exe",
    "msi",
    "msix",
    "msixbundle",
    "msp",
    "pkg",
    "rpm",
];
const VIDEO: &[&str] = &[
    "3gp", "avi", "flv", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "vob", "webm",
    "wmv",
];
const AUDIO: &[&str] = &[
    "aac", "aif", "aiff", "flac", "m4a", "mp3", "oga", "ogg", "opus", "wav", "wma",
];
const DOCUMENTS: &[&str] = &[
    "doc", "docx", "epub", "mobi", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "xls", "xlsx",
];

/// Return the normalized link when the whole text is one HTTP(S) file link.
pub fn file_link(text: &str) -> Option<String> {
    let text = text.trim();
    // One link only: a sentence or several lines containing a link is not an offer.
    if text.is_empty()
        || text.len() > MAX_TEXT_UNITS
        || text.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    let url = download_engine::model::validate_download_url(text).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // A trailing slash names a page or folder, not a file.
    let name = url.path_segments()?.next_back()?;
    let (_, extension) = name.rsplit_once('.')?;
    let extension = extension.to_ascii_lowercase();
    FILE_EXTENSIONS
        .iter()
        .any(|group| group.contains(&extension.as_str()))
        .then(|| url.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Another application put the text on the clipboard.
    Other,
    /// Download Engine itself: Copy link, Copy SHA-256, text copied from its fields.
    Own,
}

/// Decides which clipboard changes become offers. Only a keyed fingerprint of
/// the previous text is kept, never the text itself.
pub struct Filter {
    keys: RandomState,
    last: Option<u64>,
}
impl Default for Filter {
    fn default() -> Self {
        Self {
            keys: RandomState::new(),
            last: None,
        }
    }
}
impl Filter {
    /// `text` is `None` when the clipboard holds no readable text, text that is
    /// too long, or content another app asked clipboard monitors to skip.
    pub fn offer(&mut self, text: Option<&str>, source: Source) -> Option<String> {
        let Some(text) = text else {
            self.last = None;
            return None;
        };
        // Repeated Ctrl+C and apps that announce one copy several times.
        let fingerprint = self.keys.hash_one(text.trim());
        if self.last.replace(fingerprint) == Some(fingerprint) || source == Source::Own {
            return None;
        }
        file_link(text)
    }
}

#[cfg(windows)]
pub use native::Monitor;

#[cfg(windows)]
mod native {
    use super::*;
    use anyhow::{Context as _, Result, ensure};
    use std::{
        cell::{Cell, RefCell},
        ptr::{null, null_mut},
        sync::mpsc::{self, Receiver, SyncSender},
    };
    use windows::Win32::{
        Foundation::{HGLOBAL, HWND as Window},
        System::{
            DataExchange::{
                AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardOwner,
                IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
                RemoveClipboardFormatListener,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::CF_UNICODETEXT,
        },
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
        UI::WindowsAndMessaging::*,
    };
    use zeroize::Zeroizing;

    const CLASS: &str = "IDM_EX.ClipboardLinks.v1";
    const RETRY_TIMER: usize = 1;
    const RETRY_MS: u32 = 50;
    // Another clipboard reader can hold the clipboard briefly. Retry a few
    // times from the window's own timer, then give up on that copy.
    const MAX_RETRIES: u8 = 3;

    enum Content {
        Busy,
        Text(Zeroizing<String>),
        Other,
    }

    struct Context {
        filter: RefCell<Filter>,
        links: SyncSender<String>,
        repaint: eframe::egui::Context,
        skip_formats: [u32; 2],
        own: Cell<bool>,
        retries: Cell<u8>,
        reading: Cell<bool>,
    }
    impl Context {
        fn changed(&self, hwnd: HWND) {
            unsafe { KillTimer(hwnd, RETRY_TIMER) };
            self.retries.set(0);
            // Decide this when Windows reports the copy: Download Engine is in front for
            // its own Copy buttons and for text copied from its fields. egui writes
            // the clipboard without an owner window, so the owner alone is not enough.
            self.own
                .set(own_window(unsafe { GetForegroundWindow() }) || own_clipboard());
            self.inspect(hwnd);
        }
        fn inspect(&self, hwnd: HWND) {
            // Reading can wait on the copying app; a reentrant change is retried.
            let content = if self.reading.replace(true) {
                Content::Busy
            } else {
                let content = read(hwnd, &self.skip_formats);
                self.reading.set(false);
                content
            };
            let text = match content {
                Content::Busy => {
                    if self.retries.get() < MAX_RETRIES {
                        self.retries.set(self.retries.get() + 1);
                        unsafe { SetTimer(hwnd, RETRY_TIMER, RETRY_MS, None) };
                    }
                    return;
                }
                Content::Text(text) => Some(text),
                Content::Other => None,
            };
            let source = if self.own.get() {
                Source::Own
            } else {
                Source::Other
            };
            let link = self
                .filter
                .borrow_mut()
                .offer(text.as_ref().map(|text| text.as_str()), source);
            if let Some(link) = link
                && self.links.try_send(link).is_ok()
            {
                self.repaint.request_repaint();
            }
        }
    }

    fn own_window(hwnd: HWND) -> bool {
        if hwnd.is_null() {
            return false;
        }
        let mut process = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut process) };
        process != 0 && process == unsafe { GetCurrentProcessId() }
    }
    fn own_clipboard() -> bool {
        unsafe { GetClipboardOwner() }.is_ok_and(|owner| own_window(owner.0))
    }

    struct Opened;
    impl Drop for Opened {
        fn drop(&mut self) {
            let _ = unsafe { CloseClipboard() };
        }
    }
    fn read(hwnd: HWND, skip_formats: &[u32]) -> Content {
        unsafe {
            if IsClipboardFormatAvailable(CF_UNICODETEXT.0.into()).is_err()
                || skip_formats
                    .iter()
                    .any(|format| *format != 0 && IsClipboardFormatAvailable(*format).is_ok())
            {
                return Content::Other;
            }
            if OpenClipboard(Some(Window(hwnd))).is_err() {
                return Content::Busy;
            }
            let _opened = Opened;
            let Ok(handle) = GetClipboardData(CF_UNICODETEXT.0.into()) else {
                return Content::Other;
            };
            let memory = HGLOBAL(handle.0);
            // Allocations can be rounded up; the terminator search bounds the text.
            let size = GlobalSize(memory);
            if !(2..=(MAX_TEXT_UNITS + 16) * 2).contains(&size) {
                return Content::Other;
            }
            let pointer = GlobalLock(memory);
            if pointer.is_null() {
                return Content::Other;
            }
            // The HGLOBAL stays locked, and the bounded slice ends within its allocation.
            let units = std::slice::from_raw_parts(pointer.cast::<u16>(), size / 2);
            let text = units
                .iter()
                .position(|unit| *unit == 0)
                .and_then(|end| String::from_utf16(&units[..end]).ok())
                .map(Zeroizing::new);
            let _ = GlobalUnlock(memory);
            text.map_or(Content::Other, Content::Text)
        }
    }

    // The context is immutable apart from cells and stays boxed until after
    // DestroyWindow. Reading the clipboard can reenter this procedure.
    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if message == WM_NCCREATE {
                let create = &*(l as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            let context = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Context;
            if !context.is_null() {
                match message {
                    WM_CLIPBOARDUPDATE => {
                        (*context).changed(hwnd);
                        return 0;
                    }
                    WM_TIMER if w == RETRY_TIMER => {
                        KillTimer(hwnd, RETRY_TIMER);
                        (*context).inspect(hwnd);
                        return 0;
                    }
                    WM_NCDESTROY => {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    }
                    _ => {}
                }
            }
            DefWindowProcW(hwnd, message, w, l)
        }
    }

    /// A message-only window on the UI thread. The window's event loop delivers
    /// WM_CLIPBOARDUPDATE, so nothing polls the clipboard. Drop on the same thread.
    pub struct Monitor {
        hwnd: HWND,
        pub links: Receiver<String>,
        _context: Box<Context>,
    }
    impl Monitor {
        pub fn new(repaint: eframe::egui::Context) -> Result<Self> {
            let (links_tx, links) = mpsc::sync_channel(4);
            // Password managers and similar apps mark private copies with these formats.
            let skip_formats = unsafe {
                [
                    RegisterClipboardFormatW(windows::core::w!(
                        "ExcludeClipboardContentFromMonitorProcessing"
                    )),
                    RegisterClipboardFormatW(windows::core::w!("Clipboard Viewer Ignore")),
                ]
            };
            let context = Box::new(Context {
                filter: RefCell::new(Filter::default()),
                links: links_tx,
                repaint,
                skip_formats,
                own: Cell::new(false),
                retries: Cell::new(0),
                reading: Cell::new(false),
            });
            let class: Vec<u16> = CLASS.encode_utf16().chain(Some(0)).collect();
            let instance = unsafe { GetModuleHandleW(null()) };
            ensure!(
                !instance.is_null(),
                "Cannot load the clipboard window module"
            );
            let registration = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            // RegisterClass copies the name; the procedure is process-static.
            if unsafe { RegisterClassW(&registration) } == 0 {
                ensure!(
                    std::io::Error::last_os_error().raw_os_error() == Some(1410),
                    "Cannot register the clipboard window"
                );
            }
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    class.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    null_mut(),
                    instance,
                    context.as_ref() as *const Context as *const _,
                )
            };
            ensure!(!hwnd.is_null(), "Cannot create the clipboard window");
            let monitor = Self {
                hwnd,
                links,
                _context: context,
            };
            unsafe { AddClipboardFormatListener(Window(hwnd)) }
                .context("Windows did not accept the clipboard listener")?;
            Ok(monitor)
        }
    }
    impl Drop for Monitor {
        fn drop(&mut self) {
            unsafe {
                let _ = RemoveClipboardFormatListener(Window(self.hwnd));
                KillTimer(self.hwnd, RETRY_TIMER);
                DestroyWindow(self.hwnd);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn listener_window_belongs_to_this_process_and_is_destroyed_on_drop() {
            // Registers and removes real listeners; the clipboard itself is never read.
            let first = Monitor::new(eframe::egui::Context::default()).unwrap();
            let hwnd = first.hwnd;
            assert_ne!(unsafe { IsWindow(hwnd) }, 0);
            assert!(own_window(hwnd));
            assert!(!own_window(null_mut()));
            // Switching the setting off and on again reuses the registered class.
            let second = Monitor::new(eframe::egui::Context::default()).unwrap();
            assert_ne!(first.hwnd, second.hwnd);
            drop(first);
            assert_eq!(unsafe { IsWindow(hwnd) }, 0);
            let other = second.hwnd;
            drop(second);
            assert_eq!(unsafe { IsWindow(other) }, 0);
        }

        mod live {
            use super::*;
            use std::time::{Duration, Instant};
            use windows::Win32::{
                Foundation::HANDLE,
                System::{
                    DataExchange::{EmptyClipboard, EnumClipboardFormats, SetClipboardData},
                    Memory::{GMEM_MOVEABLE, GlobalAlloc},
                },
            };

            fn open(owner: Option<HWND>) -> Opened {
                // Other clipboard listeners may hold the clipboard for a moment.
                for _ in 0..100 {
                    if unsafe { OpenClipboard(owner.map(Window)) }.is_ok() {
                        return Opened;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                panic!("The clipboard stayed busy");
            }
            fn memory(bytes: &[u8]) -> HANDLE {
                unsafe {
                    let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).unwrap();
                    let pointer = GlobalLock(memory);
                    assert!(!pointer.is_null());
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
                    let _ = GlobalUnlock(memory);
                    HANDLE(memory.0)
                }
            }
            /// `owner: None` writes like egui and most apps: Windows records no owner.
            fn put(owner: Option<HWND>, text: &str, extra: &[u32]) {
                let _opened = open(owner);
                unsafe {
                    EmptyClipboard().unwrap();
                    let units: Vec<u8> = text
                        .encode_utf16()
                        .chain(Some(0))
                        .flat_map(u16::to_le_bytes)
                        .collect();
                    SetClipboardData(CF_UNICODETEXT.0.into(), Some(memory(&units))).unwrap();
                    // Keep test text out of Windows clipboard history and cloud sync.
                    for name in [
                        windows::core::w!("CanIncludeInClipboardHistory"),
                        windows::core::w!("CanUploadToCloudClipboard"),
                    ] {
                        let format = RegisterClipboardFormatW(name);
                        SetClipboardData(format, Some(memory(&0u32.to_le_bytes()))).unwrap();
                    }
                    for format in extra {
                        SetClipboardData(*format, Some(memory(&[0]))).unwrap();
                    }
                }
            }
            fn pump(monitor: &Monitor, wait: Duration) -> Vec<String> {
                let deadline = Instant::now() + wait;
                let mut links = Vec::new();
                while Instant::now() < deadline {
                    let mut message = MSG::default();
                    while unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } != 0 {
                        unsafe {
                            TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    links.extend(monitor.links.try_iter());
                    std::thread::sleep(Duration::from_millis(10));
                }
                links
            }
            /// The previous text, or `Err` when the clipboard holds data this test
            /// could not put back (images, files, rich text).
            fn saved_text() -> Result<Option<Zeroizing<String>>, u32> {
                let _opened = open(None);
                let history = unsafe {
                    [
                        RegisterClipboardFormatW(windows::core::w!("CanIncludeInClipboardHistory")),
                        RegisterClipboardFormatW(windows::core::w!("CanUploadToCloudClipboard")),
                    ]
                };
                let mut format = 0;
                loop {
                    format = unsafe { EnumClipboardFormats(format) };
                    match format {
                        0 => break,
                        // Unicode, ANSI and OEM text, the text locale, and history flags.
                        13 | 1 | 7 | 16 => {}
                        _ if history.contains(&format) => {}
                        other => return Err(other),
                    }
                }
                if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT.0.into()) }.is_err() {
                    return Ok(None);
                }
                let handle = unsafe { GetClipboardData(CF_UNICODETEXT.0.into()) }.unwrap();
                let memory = HGLOBAL(handle.0);
                unsafe {
                    let size = GlobalSize(memory);
                    let pointer = GlobalLock(memory);
                    assert!(!pointer.is_null());
                    let units = std::slice::from_raw_parts(pointer.cast::<u16>(), size / 2);
                    let end = units
                        .iter()
                        .position(|unit| *unit == 0)
                        .unwrap_or(units.len());
                    let text = Zeroizing::new(String::from_utf16_lossy(&units[..end]));
                    let _ = GlobalUnlock(memory);
                    Ok(Some(text))
                }
            }

            #[test]
            #[ignore = "Replaces the clipboard text for a few seconds, then restores it"]
            fn real_copies_are_offered_once_and_own_private_or_oversized_copies_are_not() {
                let previous = match saved_text() {
                    Ok(previous) => previous,
                    Err(format) => {
                        eprintln!("SKIPPED: the clipboard holds non-text data (format {format})");
                        return;
                    }
                };
                let monitor = Monitor::new(eframe::egui::Context::default()).unwrap();
                let link = "https://example.invalid/clipboard-test/archive-2.4.zip";
                let wait = Duration::from_millis(1500);
                let quiet = Duration::from_millis(600);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    put(None, link, &[]);
                    assert_eq!(pump(&monitor, wait), [link], "Another app's copy");
                    put(None, link, &[]);
                    assert!(pump(&monitor, quiet).is_empty(), "Repeated copy");
                    put(None, "Not a link", &[]);
                    assert!(pump(&monitor, quiet).is_empty(), "Plain text");
                    put(None, link, &[]);
                    assert_eq!(pump(&monitor, wait), [link], "Copied again later");
                    // A copy owned by a Download Engine window is its own Copy action.
                    put(Some(monitor.hwnd), "https://example.invalid/own.msi", &[]);
                    assert!(pump(&monitor, quiet).is_empty(), "Own copy");
                    let private = unsafe {
                        RegisterClipboardFormatW(windows::core::w!(
                            "ExcludeClipboardContentFromMonitorProcessing"
                        ))
                    };
                    put(None, "https://example.invalid/secret.zip", &[private]);
                    assert!(pump(&monitor, quiet).is_empty(), "Private copy");
                    let long = format!("https://example.invalid/{}.zip", "a".repeat(9000));
                    put(None, &long, &[]);
                    assert!(pump(&monitor, quiet).is_empty(), "Oversized text");
                    let other = "https://example.invalid/clipboard-test/video.mp4";
                    put(None, other, &[]);
                    assert_eq!(pump(&monitor, wait), [other], "Next real copy");
                }));
                drop(monitor);
                match previous {
                    Some(text) => put(None, &text, &[]),
                    None => {
                        let _opened = open(None);
                        unsafe { EmptyClipboard() }.unwrap();
                    }
                }
                if let Err(panic) = result {
                    std::panic::resume_unwind(panic);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_single_http_file_links_with_known_extensions_are_offered() {
        for (text, expected) in [
            (
                "https://downloads.example.com/archive-2.4.zip",
                "https://downloads.example.com/archive-2.4.zip",
            ),
            (
                "  http://example.com/Setup.EXE?token=1#part\r\n",
                "http://example.com/Setup.EXE?token=1",
            ),
            (
                "https://example.com/videos/My%20Clip.mp4",
                "https://example.com/videos/My%20Clip.mp4",
            ),
            (
                "https://example.com/report.pdf",
                "https://example.com/report.pdf",
            ),
        ] {
            assert_eq!(file_link(text).as_deref(), Some(expected), "{text}");
        }
        for text in [
            "",
            "https://example.com/",
            "https://example.com/download",
            "https://example.com/download?file=a.zip",
            "https://example.com/readme.md",
            "https://example.com/notes.txt",
            "https://example.com/index.html",
            "https://example.com/photo.jpg",
            "https://example.com/install.ps1",
            "https://example.com/live/master.m3u8",
            "https://example.com/zip",
            "https://example.com/archive.zip/",
            "ftp://example.com/archive.zip",
            "file:///C:/archive.zip",
            "https://user:secret@example.com/archive.zip",
            "Get it here: https://example.com/archive.zip",
            "https://example.com/a.zip\nhttps://example.com/b.zip",
            "https://example.com/a.zip\thttps://example.com/b.zip",
        ] {
            assert_eq!(file_link(text), None, "{text:?}");
        }
        let long = format!("https://example.com/{}.zip", "a".repeat(MAX_TEXT_UNITS));
        assert_eq!(file_link(&long), None);
    }

    #[test]
    fn repeated_copies_and_own_copies_are_ignored() {
        let mut filter = Filter::default();
        let link = "https://example.com/archive.zip";
        assert_eq!(
            filter.offer(Some(link), Source::Other).as_deref(),
            Some(link)
        );
        // The same text again, including whitespace-only differences.
        assert_eq!(filter.offer(Some(link), Source::Other), None);
        assert_eq!(
            filter.offer(Some(&format!("{link}\r\n")), Source::Other),
            None
        );
        // Other text in between: copying the link again is a new request.
        assert_eq!(filter.offer(Some("hello"), Source::Other), None);
        assert_eq!(
            filter.offer(Some(link), Source::Other).as_deref(),
            Some(link)
        );
        // Images, files and skipped private copies also count as other content.
        assert_eq!(filter.offer(None, Source::Other), None);
        assert_eq!(
            filter.offer(Some(link), Source::Other).as_deref(),
            Some(link)
        );
        // Download Engine's own Copy link is never offered back, even after other text,
        let own = "https://example.com/own.msi";
        assert_eq!(filter.offer(Some(own), Source::Own), None);
        // and copying that same text elsewhere right after it is a repeat.
        assert_eq!(filter.offer(Some(own), Source::Other), None);
        assert_eq!(filter.offer(Some("other"), Source::Own), None);
        assert_eq!(filter.offer(Some(own), Source::Other).as_deref(), Some(own));
    }
}
