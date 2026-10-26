//! Native URL drops. Registration and OLE lifetime stay on the window's STA thread.
use anyhow::{Result, ensure};
use std::collections::HashSet;
/// Drops waiting for review before further drops are refused.
pub const QUEUE: usize = 8;
pub fn urls(text: &str) -> Result<String> {
    ensure!(text.len() <= 1_000_000, "Drop at most 1 MB of URL text");
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for line in text.lines().filter(|s| !s.trim().is_empty()) {
        let url = download_engine::model::validate_download_url(line)?.to_string();
        if seen.insert(url.clone()) {
            result.push(url);
        }
        ensure!(result.len() <= 1000, "Drop at most 1,000 URLs");
    }
    ensure!(!result.is_empty(), "Drop an HTTP or HTTPS file link");
    Ok(result.join("\n"))
}
#[cfg(windows)]
pub use native::{Guard, Registration};
#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        cell::Cell,
        marker::PhantomData,
        rc::Rc,
        sync::mpsc::{self, Receiver, SyncSender},
    };
    use windows::{
        Win32::{
            Foundation::{E_INVALIDARG, HWND, POINTL},
            System::{
                Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL},
                DataExchange::RegisterClipboardFormatW,
                Memory::{GlobalLock, GlobalSize, GlobalUnlock},
                Ole::{
                    CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget,
                    IDropTarget_Impl, OleInitialize, OleUninitialize, RegisterDragDrop,
                    ReleaseStgMedium, RevokeDragDrop,
                },
                SystemServices::MODIFIERKEYS_FLAGS,
            },
        },
        core::{Ref, implement},
    };
    #[implement(IDropTarget)]
    struct Target {
        sender: SyncSender<String>,
        ctx: eframe::egui::Context,
        accepted: Cell<bool>,
        url_format: u16,
        /// Told whether a drag over the window carries links it accepts.
        hover: Option<Box<dyn Fn(bool)>>,
    }
    struct Medium(STGMEDIUM);
    impl Drop for Medium {
        fn drop(&mut self) {
            unsafe {
                ReleaseStgMedium(&mut self.0);
            }
        }
    }
    fn format(id: u16) -> FORMATETC {
        FORMATETC {
            cfFormat: id,
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
            ..Default::default()
        }
    }
    fn read(data: &IDataObject, id: u16) -> Option<String> {
        let medium = Medium(unsafe { data.GetData(&format(id)).ok()? });
        if medium.0.tymed != TYMED_HGLOBAL.0 as u32 {
            return None;
        }
        let handle = unsafe { medium.0.u.hGlobal };
        let size = unsafe { GlobalSize(handle) };
        if !(2..=2_000_002).contains(&size) || size % 2 != 0 {
            return None;
        }
        let ptr = unsafe { GlobalLock(handle) };
        if ptr.is_null() {
            return None;
        }
        // The HGLOBAL remains locked, and the bounded slice ends within its allocation.
        let value = unsafe {
            let chars = std::slice::from_raw_parts(ptr.cast::<u16>(), size / 2);
            chars
                .iter()
                .position(|c| *c == 0)
                .and_then(|n| String::from_utf16(&chars[..n]).ok())
        };
        let _ = unsafe { GlobalUnlock(handle) };
        value
    }
    impl Target {
        fn text(&self, data: Ref<IDataObject>) -> Option<String> {
            let data = data.as_ref()?;
            // Browser URL format avoids an HTML payload or a dragged link's display label.
            read(data, self.url_format)
                .or_else(|| read(data, CF_UNICODETEXT.0))
                .and_then(|s| urls(&s).ok())
        }
        fn hover(&self, accepted: bool) {
            if let Some(hover) = &self.hover {
                hover(accepted);
            }
        }
    }
    #[allow(non_snake_case)]
    impl IDropTarget_Impl for Target_Impl {
        fn DragEnter(
            &self,
            data: Ref<IDataObject>,
            _: MODIFIERKEYS_FLAGS,
            _: &POINTL,
            effect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            if effect.is_null() {
                return Err(E_INVALIDARG.into());
            }
            self.accepted.set(self.text(data).is_some());
            unsafe {
                *effect = if self.accepted.get() && effect.read().0 & DROPEFFECT_COPY.0 != 0 {
                    DROPEFFECT_COPY
                } else {
                    DROPEFFECT_NONE
                };
                self.hover(*effect == DROPEFFECT_COPY);
            }
            Ok(())
        }
        fn DragOver(
            &self,
            _: MODIFIERKEYS_FLAGS,
            _: &POINTL,
            effect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            if effect.is_null() {
                return Err(E_INVALIDARG.into());
            }
            unsafe {
                *effect = if self.accepted.get() && effect.read().0 & DROPEFFECT_COPY.0 != 0 {
                    DROPEFFECT_COPY
                } else {
                    DROPEFFECT_NONE
                };
            }
            Ok(())
        }
        fn DragLeave(&self) -> windows::core::Result<()> {
            self.accepted.set(false);
            self.hover(false);
            Ok(())
        }
        fn Drop(
            &self,
            data: Ref<IDataObject>,
            _: MODIFIERKEYS_FLAGS,
            _: &POINTL,
            effect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            if effect.is_null() {
                return Err(E_INVALIDARG.into());
            }
            let allowed = unsafe { effect.read().0 & DROPEFFECT_COPY.0 != 0 };
            let accepted = allowed
                && self
                    .text(data)
                    .is_some_and(|text| self.sender.try_send(text).is_ok());
            unsafe {
                *effect = if accepted {
                    DROPEFFECT_COPY
                } else {
                    DROPEFFECT_NONE
                };
            }
            self.accepted.set(false);
            self.hover(false);
            if accepted {
                self.ctx.request_repaint();
            }
            Ok(())
        }
    }

    /// The main window's drop target; its own bounded queue holds dropped links.
    pub struct Registration {
        pub events: Receiver<String>,
        _guard: Guard,
    }
    impl Registration {
        pub fn new(hwnd: isize, ctx: eframe::egui::Context) -> Result<Self> {
            let (sender, events) = mpsc::sync_channel(QUEUE);
            Ok(Self {
                _guard: Guard::new(hwnd, ctx, sender, None)?,
                events,
            })
        }
    }

    /// Keeps a window registered for URL drops until dropped. Create and drop it on
    /// the window's thread: OLE is initialized for that thread and released here.
    pub struct Guard {
        hwnd: HWND,
        _target: Option<IDropTarget>,
        _sta: PhantomData<Rc<()>>,
    }
    impl Guard {
        /// Accepted drops go to `sender`; a full queue refuses further drops.
        pub fn new(
            hwnd: isize,
            ctx: eframe::egui::Context,
            sender: SyncSender<String>,
            hover: Option<Box<dyn Fn(bool)>>,
        ) -> Result<Self> {
            unsafe {
                OleInitialize(None)?;
            }
            let url_format =
                unsafe { RegisterClipboardFormatW(windows::core::w!("UniformResourceLocatorW")) }
                    as u16;
            let target: IDropTarget = Target {
                sender,
                ctx,
                accepted: Cell::new(false),
                url_format,
                hover,
            }
            .into();
            let hwnd = HWND(hwnd as _);
            if let Err(error) = unsafe { RegisterDragDrop(hwnd, &target) } {
                unsafe {
                    OleUninitialize();
                }
                return Err(error.into());
            }
            Ok(Self {
                hwnd,
                _target: Some(target),
                _sta: PhantomData,
            })
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                let _ = RevokeDragDrop(self.hwnd);
                self._target.take();
                OleUninitialize();
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::{
            Win32::{
                Foundation::{DV_E_FORMATETC, E_NOTIMPL},
                System::{
                    Com::{
                        IAdviseSink, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM_0,
                    },
                    Memory::{GMEM_MOVEABLE, GlobalAlloc},
                },
            },
            core::{BOOL, HRESULT},
        };
        #[implement(IDataObject)]
        struct Data {
            text: Vec<u16>,
        }
        #[allow(non_snake_case)]
        impl IDataObject_Impl for Data_Impl {
            fn GetData(&self, requested: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
                if requested.is_null() || unsafe { (*requested).cfFormat != CF_UNICODETEXT.0 } {
                    return Err(DV_E_FORMATETC.into());
                }
                let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, self.text.len() * 2)? };
                let ptr = unsafe { GlobalLock(handle) };
                assert!(!ptr.is_null());
                unsafe {
                    std::ptr::write_bytes(ptr, 0xff, GlobalSize(handle));
                    std::ptr::copy_nonoverlapping(
                        self.text.as_ptr(),
                        ptr.cast::<u16>(),
                        self.text.len(),
                    );
                    let _ = GlobalUnlock(handle);
                }
                Ok(STGMEDIUM {
                    tymed: TYMED_HGLOBAL.0 as u32,
                    u: STGMEDIUM_0 { hGlobal: handle },
                    ..Default::default()
                })
            }
            fn GetDataHere(
                &self,
                _: *const FORMATETC,
                _: *mut STGMEDIUM,
            ) -> windows::core::Result<()> {
                Err(E_NOTIMPL.into())
            }
            fn QueryGetData(&self, _: *const FORMATETC) -> HRESULT {
                E_NOTIMPL
            }
            fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, _: *mut FORMATETC) -> HRESULT {
                E_NOTIMPL
            }
            fn SetData(
                &self,
                _: *const FORMATETC,
                _: *const STGMEDIUM,
                _: BOOL,
            ) -> windows::core::Result<()> {
                Err(E_NOTIMPL.into())
            }
            fn EnumFormatEtc(&self, _: u32) -> windows::core::Result<IEnumFORMATETC> {
                Err(E_NOTIMPL.into())
            }
            fn DAdvise(
                &self,
                _: *const FORMATETC,
                _: u32,
                _: Ref<IAdviseSink>,
            ) -> windows::core::Result<u32> {
                Err(E_NOTIMPL.into())
            }
            fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
                Err(E_NOTIMPL.into())
            }
            fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
                Err(E_NOTIMPL.into())
            }
        }
        #[test]
        fn ole_unicode_drop_uses_real_com_vtable_and_rejects_unsafe_or_unterminated_data() {
            let (sender, events) = mpsc::sync_channel(1);
            let target: IDropTarget = Target {
                sender,
                ctx: eframe::egui::Context::default(),
                accepted: Cell::new(false),
                url_format: 0,
                hover: None,
            }
            .into();
            for (text, terminated, accepted) in [
                ("https://example.com/clip.mp4", true, true),
                ("file:///C:/secret", true, false),
                ("https://example.com/file", false, false),
            ] {
                let mut text: Vec<_> = text.encode_utf16().collect();
                if terminated {
                    text.push(0);
                }
                let data: IDataObject = Data { text }.into();
                let mut effect = DROPEFFECT_COPY;
                unsafe {
                    target
                        .DragEnter(&data, MODIFIERKEYS_FLAGS(0), POINTL::default(), &mut effect)
                        .unwrap();
                }
                assert_eq!(effect == DROPEFFECT_COPY, accepted);
                // No application data is handed off while merely hovering.
                assert!(events.try_recv().is_err());
                effect = DROPEFFECT_COPY;
                unsafe {
                    target
                        .Drop(&data, MODIFIERKEYS_FLAGS(0), POINTL::default(), &mut effect)
                        .unwrap();
                }
                assert_eq!(events.try_recv().is_ok(), accepted);
                unsafe {
                    target.DragLeave().unwrap();
                }
            }
            let data: IDataObject = Data {
                text: "https://example.com/a\0".encode_utf16().collect(),
            }
            .into();
            let mut effect = DROPEFFECT_NONE;
            unsafe {
                target
                    .Drop(&data, MODIFIERKEYS_FLAGS(0), POINTL::default(), &mut effect)
                    .unwrap();
            }
            assert!(
                events.try_recv().is_err(),
                "Never honor a drop that forbids copying"
            );
            effect = DROPEFFECT_COPY;
            unsafe {
                target
                    .Drop(&data, MODIFIERKEYS_FLAGS(0), POINTL::default(), &mut effect)
                    .unwrap();
            }
            effect = DROPEFFECT_COPY;
            unsafe {
                target
                    .Drop(&data, MODIFIERKEYS_FLAGS(0), POINTL::default(), &mut effect)
                    .unwrap();
            }
            assert_eq!(
                effect, DROPEFFECT_NONE,
                "A full review queue must reject further drops"
            );
        }
        #[test]
        fn hover_lights_up_only_for_accepted_links_and_drops_reach_the_callers_queue() {
            let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
            let record = seen.clone();
            let (sender, events) = mpsc::sync_channel(1);
            let target: IDropTarget = Target {
                sender,
                ctx: eframe::egui::Context::default(),
                accepted: Cell::new(false),
                url_format: 0,
                hover: Some(Box::new(move |accepted| record.borrow_mut().push(accepted))),
            }
            .into();
            let data = |text: &str| -> IDataObject {
                Data {
                    text: text.encode_utf16().chain(Some(0)).collect(),
                }
                .into()
            };
            let links = data("https://example.com/a.zip\r\nhttps://example.com/b.zip");
            let mut effect = DROPEFFECT_COPY;
            unsafe {
                target
                    .DragEnter(
                        &links,
                        MODIFIERKEYS_FLAGS(0),
                        POINTL::default(),
                        &mut effect,
                    )
                    .unwrap();
                target.DragLeave().unwrap();
            }
            // Unsafe text, more than 1 MB and more than 1,000 links are refused.
            let oversized = format!("https://example.com/{}", "a".repeat(1_000_000));
            let many: String = (0..1001)
                .map(|i| format!("https://example.com/{i}.zip\n"))
                .collect();
            for text in ["javascript:alert(1)", &oversized, &many] {
                let refused = data(text);
                effect = DROPEFFECT_COPY;
                unsafe {
                    target
                        .DragEnter(
                            &refused,
                            MODIFIERKEYS_FLAGS(0),
                            POINTL::default(),
                            &mut effect,
                        )
                        .unwrap();
                }
                assert_eq!(effect, DROPEFFECT_NONE);
                effect = DROPEFFECT_COPY;
                unsafe {
                    target
                        .Drop(
                            &refused,
                            MODIFIERKEYS_FLAGS(0),
                            POINTL::default(),
                            &mut effect,
                        )
                        .unwrap();
                }
                assert_eq!(effect, DROPEFFECT_NONE);
            }
            effect = DROPEFFECT_COPY;
            unsafe {
                target
                    .DragEnter(
                        &links,
                        MODIFIERKEYS_FLAGS(0),
                        POINTL::default(),
                        &mut effect,
                    )
                    .unwrap();
                target
                    .Drop(
                        &links,
                        MODIFIERKEYS_FLAGS(0),
                        POINTL::default(),
                        &mut effect,
                    )
                    .unwrap();
            }
            assert_eq!(effect, DROPEFFECT_COPY);
            assert_eq!(
                events.try_recv().unwrap(),
                "https://example.com/a.zip\nhttps://example.com/b.zip"
            );
            assert!(events.try_recv().is_err());
            let mut expected = vec![true, false];
            expected.extend([false; 6]);
            expected.extend([true, false]);
            assert_eq!(*seen.borrow(), expected);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drops_are_validated_deduplicated_and_never_interpreted_as_commands() {
        assert_eq!(
            urls("https://example.com/a#one\r\nhttps://example.com/a#two").unwrap(),
            "https://example.com/a"
        );
        for value in [
            "",
            "file:///C:/secret",
            "javascript:alert(1)",
            "https://user:pass@example.com/a",
            "https://example.com/a\ncmd.exe",
        ] {
            assert!(urls(value).is_err());
        }
        assert!(urls(&"x".repeat(1_000_001)).is_err());
    }
}
