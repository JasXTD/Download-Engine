//! Launch Services activation without replacing winit's application delegate.
//! Apple events request the same bounded torrent review as a command-line launch.
use anyhow::{Context, Result, ensure};
use download_engine::{external_open::Target, store::Store};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{NSObject, NSObjectProtocol},
    sel,
};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};

const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const URL_EVENT: u32 = u32::from_be_bytes(*b"GURL");
const CORE_EVENT: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");

struct State {
    store: Store,
}
define_class!(
    #[unsafe(super = NSObject)]
    #[name = "DownloadEngineOpenEventHandler"]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    struct Handler;
    unsafe impl NSObjectProtocol for Handler {}
    impl Handler {
        #[unsafe(method(handleURL:withReplyEvent:))]
        fn url(&self, event: &NSAppleEventDescriptor, reply: &NSAppleEventDescriptor) {
            self.receive(event, reply, false);
        }
        #[unsafe(method(handleDocuments:withReplyEvent:))]
        fn documents(&self, event: &NSAppleEventDescriptor, reply: &NSAppleEventDescriptor) {
            self.receive(event, reply, true);
        }
    }
);

impl Handler {
    fn receive(&self, event: &NSAppleEventDescriptor, reply: &NSAppleEventDescriptor, files: bool) {
        // Rust unwinding must never cross an Objective-C event callback.
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.parse(event, files)));
        if !matches!(result, Ok(Ok(()))) {
            // errAEEventNotHandled: let Launch Services report failure. Do not log
            // the document/URL, which could contain private tracker parameters.
            let error = NSAppleEventDescriptor::descriptorWithInt32(-1708);
            // These selectors use Apple's AEKeyword typedef (UInt32). Using its
            // ABI here avoids adding the otherwise-unused Core Services crate.
            unsafe {
                let _: () = msg_send![reply, setParamDescriptor: &*error, forKeyword: u32::from_be_bytes(*b"errn")];
            }
        }
    }
    fn parse(&self, event: &NSAppleEventDescriptor, files: bool) -> Result<()> {
        let value: Option<Retained<NSAppleEventDescriptor>> =
            unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
        let value = value.context("Missing open event")?;
        let mut targets = Vec::new();
        if files {
            let count = value.numberOfItems();
            ensure!(
                (1..=download_engine::external_open::MAX_PENDING as isize).contains(&count),
                "Too many torrent files"
            );
            for index in 1..=count {
                let item = value
                    .descriptorAtIndex(index)
                    .context("Missing file descriptor")?;
                let url: Option<Retained<NSAppleEventDescriptor>> = unsafe {
                    msg_send![&*item, coerceToDescriptorType: u32::from_be_bytes(*b"furl")]
                };
                let data = url.context("Expected a local file URL")?.data();
                ensure!(data.len() <= 32768, "File URL is too long");
                let text = String::from_utf8(data.to_vec())?;
                targets.push(Target::parse(
                    text.as_ref(),
                    &self.ivars().store.paths.downloads,
                )?);
            }
        } else {
            let text = value.stringValue().context("Missing magnet link")?;
            ensure!(text.len() <= 32768, "Magnet link is too long");
            targets.push(Target::parse(
                text.to_string().as_ref(),
                &self.ivars().store.paths.downloads,
            )?);
        }
        self.ivars().store.submit_external_opens(&targets)?;
        Ok(())
    }
}

pub(crate) struct Registration {
    manager: Retained<NSAppleEventManager>,
    _handler: Retained<Handler>,
}
impl Registration {
    pub(crate) fn new(store: Store) -> Result<Self> {
        let main = MainThreadMarker::new().context("macOS open events require the main thread")?;
        let allocated = Handler::alloc(main).set_ivars(State { store });
        // Ivars are initialized above; initialize the NSObject superclass so
        // objc2 can finish ownership of this partially initialized subclass.
        let handler: Retained<Handler> = unsafe { msg_send![super(allocated), init] };
        let manager = NSAppleEventManager::sharedAppleEventManager();
        // The selectors above have the documented event/reply Objective-C ABI.
        // Keep the handler alive until registration is removed in Drop.
        unsafe {
            let _: () = msg_send![&*manager, setEventHandler: &*handler, andSelector: sel!(handleURL:withReplyEvent:), forEventClass: URL_EVENT, andEventID: URL_EVENT];
            let _: () = msg_send![&*manager, setEventHandler: &*handler, andSelector: sel!(handleDocuments:withReplyEvent:), forEventClass: CORE_EVENT, andEventID: OPEN_DOCUMENTS];
        }
        Ok(Self {
            manager,
            _handler: handler,
        })
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.manager, removeEventHandlerForEventClass: URL_EVENT, andEventID: URL_EVENT];
            let _: () = msg_send![&*self.manager, removeEventHandlerForEventClass: CORE_EVENT, andEventID: OPEN_DOCUMENTS];
        }
    }
}
