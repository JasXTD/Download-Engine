//! Windows tray owns its message loop and native resources on one thread.
//! The window procedure only queues events: Shell/menu APIs may reenter it.
use download_engine::{
    i18n::{self, Language, tr},
    notifications::{LinkOffer, Notice, ScriptNotice},
    paths::AppPaths,
};
use eframe::egui;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Open,
    BrowserSetup,
    Settings,
    PauseAll,
    Exit,
    Review(String),
    ReviewQueue(String),
    /// A script notification was clicked: open that run in Run logs.
    ScriptRun(String),
    /// The copied-link notification was clicked; the link itself stays in the app.
    ClipboardLink,
    /// Show or hide the floating drop box.
    DropTarget,
    Unavailable(String),
}
fn instance_name(paths: &AppPaths) -> anyhow::Result<String> {
    let mut hash = DefaultHasher::new();
    // Only a show-window signal crosses processes. Never accept paths, URLs,
    // or download commands through these window messages.
    std::fs::canonicalize(&paths.data)?
        .to_string_lossy()
        .to_lowercase()
        .hash(&mut hash);
    Ok(format!("IDM_EX.Tray.{:016x}", hash.finish()))
}
pub fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/download-engine.png"))
        .expect("The embedded Download Engine window icon must be a valid PNG")
}
#[cfg(windows)]
pub use windows::{Tray, show_existing};
#[cfg(not(windows))]
pub struct Tray;
#[cfg(not(windows))]
impl Tray {
    pub fn new(_: &AppPaths, _: egui::Context) -> anyhow::Result<Self> {
        anyhow::bail!("The notification area is supported on Windows.")
    }
    pub fn available(&self) -> bool {
        false
    }
    pub fn events(&self) -> Vec<Action> {
        Vec::new()
    }
    pub fn notify(&self, _: Notice) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn offer_link(&self, _: LinkOffer) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn notify_script(&self, _: ScriptNotice) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn clear_notifications(&self) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn set_drop_target(&self, _: bool) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn set_language(&self, _: Language) -> anyhow::Result<()> {
        Ok(())
    }
}
#[cfg(not(windows))]
pub fn show_existing(_: &AppPaths, _: bool) -> anyhow::Result<bool> {
    Ok(false)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use anyhow::{Context, Result, ensure};
    use download_engine::notifications::Notices;
    use std::{
        mem::size_of,
        ptr::{null, null_mut},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{Shell::*, WindowsAndMessaging::*},
    };
    const CLASS: &str = "IDM_EX.NotificationArea.v1";
    const CALLBACK: u32 = WM_APP + 1;
    const WAKE: u32 = WM_APP + 2;
    const ACTIVATE: u32 = WM_APP + 3;
    const OPEN: usize = 1;
    const PAUSE: usize = 2;
    const OPTIONS: usize = 3;
    const EXIT: usize = 4;
    const DROP_TARGET: usize = 5;
    enum Command {
        Language(Language),
        Notify(Notice),
        Link(LinkOffer),
        Script(ScriptNotice),
        Clear,
        /// Check or uncheck the floating drop box menu item.
        DropTarget(bool),
        Stop,
        #[cfg(test)]
        Snapshot(mpsc::Sender<(u32, Option<String>)>),
    }
    struct Callback {
        message: u32,
        w: usize,
        l: isize,
    }
    struct WindowContext {
        events: mpsc::Sender<Callback>,
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn text<const N: usize>(target: &mut [u16; N], value: &str) {
        target.fill(0);
        let mut index = 0;
        for ch in value.chars().filter(|c| !c.is_control() || *c == '\n') {
            let mut encoded = [0; 2];
            let units = ch.encode_utf16(&mut encoded);
            if index + units.len() >= N {
                break;
            }
            target[index..index + units.len()].copy_from_slice(units);
            index += units.len();
        }
    }
    // The context is immutable and remains boxed until after DestroyWindow.
    // Queuing callbacks avoids a mutable Rust borrow across reentrant Win32 calls.
    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if message == WM_NCCREATE {
                let create = &*(l as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowContext;
            if !pointer.is_null() {
                let _ = (*pointer).events.send(Callback { message, w, l });
            }
            if message == WM_NCDESTROY {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            if message == WM_CLOSE {
                EndMenu();
                return 0;
            }
            if message == WM_QUERYENDSESSION {
                return 1;
            }
            DefWindowProcW(hwnd, message, w, l)
        }
    }
    struct Native {
        hwnd: HWND,
        icon: HICON,
        menu: HMENU,
        generation: u32,
        _context: Box<WindowContext>,
    }
    impl Drop for Native {
        fn drop(&mut self) {
            // Only our own icon/window/menu/owned icon image are released here.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &self.data());
                KillTimer(self.hwnd, 1);
                DestroyWindow(self.hwnd);
                DestroyMenu(self.menu);
                DestroyIcon(self.icon);
            }
        }
    }
    impl Native {
        fn new(name: &str, events: mpsc::Sender<Callback>) -> Result<Self> {
            let context = Box::new(WindowContext { events });
            let class = wide(CLASS);
            let title = wide(name);
            // RegisterClass copies the class name; the procedure is process-static.
            let instance = unsafe { GetModuleHandleW(null()) };
            ensure!(!instance.is_null(), "Cannot load the tray window module");
            let registration = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            if unsafe { RegisterClassW(&registration) } == 0 {
                ensure!(
                    std::io::Error::last_os_error().raw_os_error() == Some(1410),
                    "Cannot register the tray window"
                );
            }
            // A hidden top-level window receives Explorer's TaskbarCreated broadcast;
            // a message-only window would not receive it.
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    class.as_ptr(),
                    title.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    null_mut(),
                    null_mut(),
                    instance,
                    context.as_ref() as *const _ as *const _,
                )
            };
            ensure!(!hwnd.is_null(), "Cannot create the tray window");
            let mut native = Self {
                hwnd,
                icon: null_mut(),
                menu: null_mut(),
                generation: 1,
                _context: context,
            };
            let image = include_bytes!("../assets/download-engine-32.png");
            // Let Windows decode the PNG alpha channel. This returns an owned
            // HICON, released by Native::drop along with the other tray resources.
            native.icon = unsafe {
                CreateIconFromResourceEx(
                    image.as_ptr(),
                    image.len() as u32,
                    1,
                    0x0003_0000,
                    32,
                    32,
                    LR_DEFAULTCOLOR,
                )
            };
            ensure!(!native.icon.is_null(), "Cannot create the tray icon");
            native.menu = unsafe { CreatePopupMenu() };
            ensure!(!native.menu.is_null(), "Cannot create the tray menu");
            for (id, label) in [
                (OPEN, "Open Download Engine"),
                (PAUSE, "Pause all downloads"),
                (DROP_TARGET, "Floating drop box"),
                (OPTIONS, "Preferences"),
                (EXIT, "Exit"),
            ] {
                let label = wide(tr(label));
                ensure!(
                    unsafe { AppendMenuW(native.menu, MF_STRING, id, label.as_ptr()) } != 0,
                    "Cannot create a tray menu item"
                );
            }
            ensure!(
                unsafe { SetTimer(hwnd, 1, 500, None) } != 0,
                "Cannot start tray recovery timer"
            );
            Ok(native)
        }
        fn data(&self) -> NOTIFYICONDATAW {
            NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: self.generation,
                ..Default::default()
            }
        }
        fn add(&self) -> Result<()> {
            let mut data = self.data();
            data.hIcon = self.icon;
            data.uCallbackMessage = CALLBACK;
            data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
            text(&mut data.szTip, "Download Engine — download manager");
            ensure!(
                unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0,
                "Windows notification area is unavailable"
            );
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            if unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) } == 0 {
                unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
                anyhow::bail!("Windows could not enable tray interactions");
            }
            Ok(())
        }
        fn reset(&mut self) -> Result<()> {
            unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data()) };
            // Old balloon/menu callbacks must not refer to a new notification.
            self.generation = self.generation % u16::MAX as u32 + 1;
            self.add()
        }
        fn notify(&self, notice: &Notice) -> Result<()> {
            self.balloon(&notice.title(), &notice.text(), notice.failed > 0)
        }
        fn balloon(&self, title: &str, body: &str, warning: bool) -> Result<()> {
            let mut data = self.data();
            data.uFlags = NIF_INFO;
            data.dwInfoFlags = NIIF_RESPECT_QUIET_TIME
                | NIIF_NOSOUND
                | if warning { NIIF_WARNING } else { NIIF_INFO };
            text(&mut data.szInfoTitle, title);
            text(&mut data.szInfo, body);
            ensure!(
                unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0,
                "Windows could not show the download notification"
            );
            Ok(())
        }
        fn context_menu(&self, w: usize) -> Option<Action> {
            let x = (w as u16 as i16) as i32;
            let y = ((w >> 16) as u16 as i16) as i32;
            let id = unsafe {
                SetForegroundWindow(self.hwnd);
                let id = TrackPopupMenu(
                    self.menu,
                    TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                    x,
                    y,
                    0,
                    self.hwnd,
                    null(),
                );
                PostMessageW(self.hwnd, WM_NULL, 0, 0);
                Shell_NotifyIconW(NIM_SETFOCUS, &self.data());
                id as usize
            };
            menu_action(id)
        }
    }
    fn menu_action(id: usize) -> Option<Action> {
        match id {
            OPEN => Some(Action::Open),
            PAUSE => Some(Action::PauseAll),
            OPTIONS => Some(Action::Settings),
            DROP_TARGET => Some(Action::DropTarget),
            EXIT => Some(Action::Exit),
            _ => None,
        }
    }
    pub struct Tray {
        hwnd: usize,
        commands: mpsc::Sender<Command>,
        events: mpsc::Receiver<Action>,
        available: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }
    impl Tray {
        pub fn new(paths: &AppPaths, ctx: egui::Context) -> Result<Self> {
            let language = i18n::current();
            let name = instance_name(paths)?;
            let (commands, rx) = mpsc::channel();
            let (events_tx, events) = mpsc::channel();
            let (ready_tx, ready) = mpsc::sync_channel(1);
            let available = Arc::new(AtomicBool::new(false));
            let status = available.clone();
            let thread = thread::Builder::new().name("idm-ex-tray".into()).spawn(move || {
                let mut language_scope = i18n::scope(language);
                let emit = |action| {
                    let _ = events_tx.send(action);
                    ctx.request_repaint();
                };
                let (callback_tx, callbacks) = mpsc::channel();
                let result = Native::new(&name, callback_tx).and_then(|native| {
                    native.add()?;
                    Ok(native)
                });
                let mut native = match result {
                    Ok(native) => native,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("{error:#}")));
                        return;
                    }
                };
                status.store(true, Ordering::Release);
                if ready_tx.send(Ok(native.hwnd as usize)).is_err() { return; }
                let taskbar_created = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
                let mut notices = Notices::default();
                let mut last_retry = Instant::now();
                let mut drop_target = false;
                #[cfg(test)]
                let mut snapshot_sender: Option<mpsc::Sender<(u32, Option<String>)>> = None;
                loop {
                    let mut message = MSG::default();
                    if unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } <= 0 {
                        emit(Action::Unavailable("The notification area stopped. Keep this window open or restart Download Engine.".into()));
                        break;
                    }
                    unsafe { TranslateMessage(&message); DispatchMessageW(&message); }
                    #[cfg(test)]
                    let mut snapshot_requested = false;
                    let mut reset = false;
                    let mut stopping = false;
                    for command in rx.try_iter() {
                        match command {
                            Command::Language(language) => {
                                drop(language_scope);
                                language_scope = i18n::scope(language);
                                for (id, key) in [(OPEN,"Open Download Engine"),(PAUSE,"Pause all downloads"),(DROP_TARGET,"Floating drop box"),(OPTIONS,"Preferences"),(EXIT,"Exit")] {
                                    // Replacing an item clears its check mark unless it is passed again.
                                    let checked = if id == DROP_TARGET && drop_target { MF_CHECKED } else { MF_UNCHECKED };
                                    if unsafe { ModifyMenuW(native.menu, id as u32, MF_BYCOMMAND | MF_STRING | checked, id, wide(tr(key)).as_ptr()) } == 0 {
                                        emit(Action::Unavailable(std::io::Error::last_os_error().to_string()));
                                    }
                                }
                            }
                            Command::Stop => stopping = true,
                            Command::Notify(notice) => notices.push(notice),
                            Command::Link(link) => notices.push_link(link),
                            Command::Script(notice) => notices.push_script(notice),
                            Command::Clear => { notices.clear(); reset = true; }
                            Command::DropTarget(on) => {
                                drop_target = on;
                                let state = MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED };
                                // CheckMenuItem returns u32::MAX when the item is missing.
                                if unsafe { CheckMenuItem(native.menu, DROP_TARGET as u32, state) } == u32::MAX {
                                    emit(Action::Unavailable("Cannot update the tray menu".into()));
                                }
                            }
                            #[cfg(test)]
                            Command::Snapshot(sender) => snapshot_sender = Some(sender),
                        }
                    }
                    if stopping { break; }
                    for event in callbacks.try_iter() {
                        #[cfg(test)]
                        if event.message == WM_APP + 4 { snapshot_requested = true; }
                        let mut action = None;
                        if event.message == ACTIVATE {
                            action = Some(if event.w == 1 { Action::BrowserSetup } else { Action::Open });
                        } else if event.message == WM_COMMAND {
                            action = menu_action(event.w & 0xffff);
                        } else if event.message == WM_CLOSE || (event.message == WM_ENDSESSION && event.w != 0) {
                            action = Some(Action::Exit);
                        } else if event.message == taskbar_created && taskbar_created != 0 {
                            notices.clear();
                            reset = true;
                        } else if event.message == CALLBACK && ((event.l as u32) >> 16) == native.generation && !reset {
                            match event.l as u32 & 0xffff {
                                NIN_SELECT | 1025 => action = Some(Action::Open), // NIN_KEYSELECT = NIN_SELECT | NINF_KEY.
                                WM_CONTEXTMENU => action = native.context_menu(event.w),
                                NIN_BALLOONUSERCLICK => {
                                    action = if notices.dismiss_link() {
                                        Some(Action::ClipboardLink)
                                    } else if let Some(script) = notices.dismiss_script() {
                                        Some(Action::ScriptRun(script.run))
                                    } else {
                                        notices.dismiss().map(|n| match n.queue { Some(id) => Action::ReviewQueue(id), None => Action::Review(n.job) })
                                    };
                                    reset = true;
                                }
                                NIN_BALLOONHIDE | NIN_BALLOONTIMEOUT => {
                                    notices.dismiss_link();
                                    notices.dismiss_script();
                                    notices.dismiss();
                                    reset = true;
                                }
                                _ => {}
                            }
                        }
                        if let Some(action) = action { emit(action); }
                    }
                    // Retire this icon identity before displaying another balloon.
                    // Windows 10 may retain a timed-out banner in Notification Center;
                    // a delayed click must never select the next download instead.
                    if reset {
                        match native.reset() {
                            Ok(()) => status.store(true, Ordering::Release),
                            Err(error) => {
                                notices.clear();
                                status.store(false, Ordering::Release);
                                emit(Action::Unavailable(error.to_string()));
                                last_retry = Instant::now();
                            }
                        }
                    }
                    if !status.load(Ordering::Acquire) && last_retry.elapsed() >= Duration::from_secs(5) {
                        last_retry = Instant::now();
                        if native.reset().is_ok() {
                            status.store(true, Ordering::Release);
                            ctx.request_repaint();
                        }
                    }
                    let shown = if !status.load(Ordering::Acquire) {
                        None
                    } else if let Some(link) = notices.next_link() {
                        Some(native.balloon(&link.title(), &link.text(), false))
                    } else if let Some(notice) = notices.next_notice() {
                        Some(native.notify(notice))
                    } else {
                        notices.next_script().map(|script| native.balloon(&script.title, &script.text(), false))
                    };
                    if let Some(Err(error)) = shown {
                        notices.clear();
                        status.store(false, Ordering::Release);
                        emit(Action::Unavailable(error.to_string()));
                        last_retry = Instant::now();
                    }
                    #[cfg(test)]
                    if snapshot_requested && let Some(sender) = snapshot_sender.take() {
                        let _ = sender.send((native.generation, notices.active.as_ref().map(|n| n.job.clone())));
                    }
                }
                status.store(false, Ordering::Release);
                // Native resources are destroyed on the same thread that created them.
            })?;
            match ready.recv().context("Tray initialization stopped")? {
                Ok(hwnd) => Ok(Self {
                    hwnd,
                    commands,
                    events,
                    available,
                    thread: Some(thread),
                }),
                Err(error) => {
                    let _ = thread.join();
                    anyhow::bail!(error)
                }
            }
        }
        fn send(&self, command: Command) -> Result<()> {
            self.commands
                .send(command)
                .context("The notification area is unavailable")?;
            ensure!(
                unsafe { PostMessageW(self.hwnd as HWND, WAKE, 0, 0) } != 0,
                "Cannot wake the notification area"
            );
            Ok(())
        }
        pub fn available(&self) -> bool {
            self.available.load(Ordering::Acquire)
        }
        pub fn events(&self) -> Vec<Action> {
            self.events.try_iter().collect()
        }
        pub fn notify(&self, notice: Notice) -> Result<()> {
            self.send(Command::Notify(notice))
        }
        pub fn offer_link(&self, link: LinkOffer) -> Result<()> {
            self.send(Command::Link(link))
        }
        pub fn notify_script(&self, notice: ScriptNotice) -> Result<()> {
            self.send(Command::Script(notice))
        }
        pub fn set_language(&self, language: Language) -> Result<()> {
            self.send(Command::Language(language))
        }
        pub fn clear_notifications(&self) -> Result<()> {
            self.send(Command::Clear)
        }
        pub fn set_drop_target(&self, on: bool) -> Result<()> {
            self.send(Command::DropTarget(on))
        }
    }
    impl Drop for Tray {
        fn drop(&mut self) {
            let _ = self.send(Command::Stop);
            // Dismiss a menu being tracked on the tray thread before joining it.
            unsafe { PostMessageW(self.hwnd as HWND, WM_CANCELMODE, 0, 0) };
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
    pub fn show_existing(paths: &AppPaths, browser_setup: bool) -> Result<bool> {
        let class = wide(CLASS);
        let title = wide(&instance_name(paths)?);
        let hwnd = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        if hwnd.is_null() {
            return Ok(false);
        }
        ensure!(
            unsafe { PostMessageW(hwnd, ACTIVATE, usize::from(browser_setup), 0) } != 0,
            "Cannot show the running Download Engine window"
        );
        Ok(true)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn notification_text_is_bounded_terminated_and_does_not_split_unicode() {
            let mut buffer = [0; 8];
            text(&mut buffer, "😀😀😀😀\0hidden");
            assert_eq!(String::from_utf16(&buffer[..6]).unwrap(), "😀😀😀");
            assert_eq!(&buffer[6..], &[0, 0]);
            text(&mut buffer, "a\0b\tc");
            assert_eq!(&buffer[..4], &[97, 98, 99, 0]);
        }
        #[test]
        fn tray_menu_actions_are_explicit_and_ignore_unknown_commands() {
            assert_eq!(menu_action(OPEN), Some(Action::Open));
            assert_eq!(menu_action(PAUSE), Some(Action::PauseAll));
            assert_eq!(menu_action(EXIT), Some(Action::Exit));
            assert_eq!(menu_action(DROP_TARGET), Some(Action::DropTarget));
            assert_eq!(menu_action(999), None);
        }

        #[test]
        #[ignore = "Creates an isolated real Windows tray icon and transient test notifications"]
        fn native_notifications_route_clicks_retire_old_ids_and_remove_resources() {
            fn snapshot(tray: &Tray) -> (u32, Option<String>) {
                let (send, recv) = mpsc::channel();
                tray.send(Command::Snapshot(send)).unwrap();
                // A Win32 message is the barrier for earlier callbacks; the command
                // channel can be drained before their messages have been dispatched.
                post(tray, WM_APP + 4, 0, 0);
                recv.recv_timeout(Duration::from_secs(5)).unwrap()
            }
            fn post(tray: &Tray, message: u32, w: usize, l: isize) {
                assert_ne!(unsafe { PostMessageW(tray.hwnd as HWND, message, w, l) }, 0);
            }
            let root = tempfile::tempdir().unwrap();
            let paths = AppPaths {
                data: root.path().join("data"),
                downloads: root.path().join("files"),
            };
            paths.prepare().unwrap();
            let tray = Tray::new(&paths, egui::Context::default()).unwrap();
            assert!(tray.available());
            let (first, _) = snapshot(&tray);
            tray.notify(Notice {
                queue: None,
                job: "first".into(),
                name: "Download Engine notification test".into(),
                completed: 1,
                failed: 0,
            })
            .unwrap();
            assert_eq!(snapshot(&tray).1.as_deref(), Some("first"));
            post(
                &tray,
                CALLBACK,
                0,
                ((first << 16) | NIN_BALLOONUSERCLICK) as isize,
            );
            let (second, active) = snapshot(&tray);
            assert!(active.is_none());
            assert_ne!(first, second);
            assert_eq!(tray.events(), vec![Action::Review("first".into())]);
            tray.notify(Notice {
                queue: None,
                job: "second".into(),
                name: "Download Engine second test".into(),
                completed: 0,
                failed: 1,
            })
            .unwrap();
            assert_eq!(snapshot(&tray).1.as_deref(), Some("second"));
            post(
                &tray,
                CALLBACK,
                0,
                ((first << 16) | NIN_BALLOONUSERCLICK) as isize,
            );
            assert_eq!(snapshot(&tray).1.as_deref(), Some("second"));
            assert!(tray.events().is_empty());
            post(
                &tray,
                CALLBACK,
                0,
                ((second << 16) | NIN_BALLOONUSERCLICK) as isize,
            );
            snapshot(&tray);
            assert_eq!(tray.events(), vec![Action::Review("second".into())]);
            tray.notify(Notice {
                queue: Some("queue-test".into()),
                job: String::new(),
                name: "Download Engine queue completion test".into(),
                completed: 1,
                failed: 0,
            })
            .unwrap();
            let (generation, active) = snapshot(&tray);
            assert_eq!(active.as_deref(), Some(""));
            post(
                &tray,
                CALLBACK,
                0,
                ((generation << 16) | NIN_BALLOONUSERCLICK) as isize,
            );
            snapshot(&tray);
            assert_eq!(
                tray.events(),
                vec![Action::ReviewQueue("queue-test".into())]
            );
            tray.offer_link(LinkOffer {
                name: "Download Engine copied link test.zip".into(),
                host: "example.invalid".into(),
            })
            .unwrap();
            let (generation, active) = snapshot(&tray);
            assert!(active.is_none(), "A link offer is not a download target");
            post(
                &tray,
                CALLBACK,
                0,
                ((generation << 16) | NIN_BALLOONUSERCLICK) as isize,
            );
            snapshot(&tray);
            assert_eq!(tray.events(), vec![Action::ClipboardLink]);
            post(&tray, WM_COMMAND, PAUSE, 0);
            snapshot(&tray);
            assert_eq!(tray.events(), vec![Action::PauseAll]);
            assert!(show_existing(&paths, true).unwrap());
            snapshot(&tray);
            assert_eq!(tray.events(), vec![Action::BrowserSetup]);
            let hwnd = tray.hwnd;
            drop(tray);
            assert_eq!(unsafe { IsWindow(hwnd as HWND) }, 0);
            assert!(!show_existing(&paths, false).unwrap());
        }
    }
}
