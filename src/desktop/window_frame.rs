//! Keep the Windows caption and its painted pixels in the same theme as the UI.
use super::Desktop;
use anyhow::{Context, Result, ensure};
use download_engine::i18n::tr;
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{mem::size_of, ptr::null};
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::{
        Dwm::{
            DWMWA_CLOAK, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmGetWindowAttribute,
            DwmSetWindowAttribute,
        },
        Gdi::{RDW_ERASENOW, RDW_FRAME, RDW_INVALIDATE, RDW_NOCHILDREN, RedrawWindow},
    },
};

/// Cloaked windows are not drawn or shown on the taskbar. A sign-in launch cloaks
/// the window because eframe shows it after the first frame, before it can be hidden.
pub(super) fn set_cloaked(owner: &impl HasWindowHandle, cloaked: bool) -> Result<()> {
    let handle = owner.window_handle()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        anyhow::bail!(tr("Expected a Windows desktop window"));
    };
    let value = i32::from(cloaked);
    let result = unsafe {
        DwmSetWindowAttribute(
            handle.hwnd.get() as HWND,
            DWMWA_CLOAK as u32,
            (&value as *const i32).cast(),
            size_of::<i32>() as u32,
        )
    };
    ensure!(
        result >= 0,
        "Cannot change window cloaking (0x{result:08X})"
    );
    Ok(())
}

#[derive(Default)]
pub(super) struct State {
    applied: Option<(isize, bool)>,
    unavailable: bool,
}

fn dark_mode(hwnd: HWND) -> Result<bool> {
    let mut value = 0_i32;
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&mut value as *mut i32).cast(),
            size_of::<i32>() as u32,
        )
    };
    ensure!(
        result >= 0,
        "Cannot read the title-bar theme (0x{result:08X})"
    );
    Ok(value != 0)
}

impl State {
    fn apply(&mut self, owner: &impl HasWindowHandle, dark: bool) -> Result<()> {
        // Borrow the current handle for this call; never retain a native window or
        // invoke APIs from a worker thread or after eframe has destroyed it.
        let handle = owner.window_handle()?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            anyhow::bail!(tr("Expected a Windows desktop window"));
        };
        let hwnd = handle.hwnd.get() as HWND;
        let key = (handle.hwnd.get(), dark);
        // Windows/winit can reset the caption on an OS settings notification,
        // including while our app has a fixed Light/Dark preference.
        if self.applied == Some(key) && dark_mode(hwnd)? == dark {
            return Ok(());
        }
        let value = i32::from(dark);
        let result = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                (&value as *const i32).cast(),
                size_of::<i32>() as u32,
            )
        };
        ensure!(
            result >= 0,
            "Cannot set the title-bar theme (0x{result:08X})"
        );
        // Refresh the non-client frame now. ERASENOW avoids synchronously
        // re-entering the client renderer through WM_PAINT (UPDATENOW would).
        let painted = unsafe {
            RedrawWindow(
                hwnd,
                null(),
                std::ptr::null_mut(),
                RDW_FRAME | RDW_INVALIDATE | RDW_ERASENOW | RDW_NOCHILDREN,
            )
        };
        if painted == 0 {
            return Err(std::io::Error::last_os_error())
                .context(tr("Cannot repaint the title bar"));
        }
        refresh_caption(hwnd);
        self.applied = Some(key);
        Ok(())
    }
}

/// Windows 10 keeps drawing a visible caption in its old theme until its activation
/// state is repainted; a frame redraw alone does not do it. Repaint the caption
/// inactive and back to its real state through the default handler, so winit (and
/// egui) never see a focus change and keyboard state is untouched.
fn refresh_caption(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GetWindowInfo, IsWindowVisible, WINDOWINFO, WM_NCACTIVATE, WS_ACTIVECAPTION,
    };
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        // A hidden window takes the new theme when it is next shown.
        return;
    }
    let mut info = WINDOWINFO {
        cbSize: size_of::<WINDOWINFO>() as u32,
        ..Default::default()
    };
    let active =
        unsafe { GetWindowInfo(hwnd, &mut info) } != 0 && info.dwWindowStatus == WS_ACTIVECAPTION;
    unsafe {
        DefWindowProcW(hwnd, WM_NCACTIVATE, usize::from(!active), 0);
        DefWindowProcW(hwnd, WM_NCACTIVATE, usize::from(active), 0);
    }
}

impl Desktop {
    pub fn sync_window_frame(&mut self, ctx: &egui::Context, owner: &impl HasWindowHandle) {
        if self.window_frame.unavailable {
            return;
        }
        if let Err(error) = self
            .window_frame
            .apply(owner, ctx.theme() == egui::Theme::Dark)
        {
            // Older Windows versions may not support this DWM attribute. Keep
            // eframe's standard decorations and log the failure once.
            self.window_frame.unavailable = true;
            ctx.options_mut(|options| options.sync_window_theme = true);
            download_engine::diagnostics::record(
                download_engine::diagnostics::Level::Warn,
                download_engine::diagnostics::Event::UiError,
                download_engine::diagnostics::Details {
                    failure: Some(download_engine::diagnostics::Failure::error(&error)),
                    ..Default::default()
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raw_window_handle::{HandleError, Win32WindowHandle, WindowHandle};
    use std::{cell::Cell, num::NonZeroIsize, ptr::null_mut};
    use windows_sys::Win32::{
        Foundation::{LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    };

    // An owned, off-screen window exercises the real DWM/non-client paint path
    // without changing the user's theme, foreground window, or running app.
    struct Window {
        hwnd: HWND,
        class: Vec<u16>,
        paints: Box<Cell<usize>>,
    }
    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if message == WM_NCCREATE {
                let create = &*(l as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            if message == WM_NCPAINT {
                let paints = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Cell<usize>;
                if !paints.is_null() {
                    (*paints).set((*paints).get() + 1);
                }
            }
            DefWindowProcW(hwnd, message, w, l)
        }
    }
    impl Window {
        fn new() -> Self {
            let class: Vec<_> = format!("Download Engine.TitleThemeTest.{}", uuid::Uuid::new_v4())
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let paints = Box::new(Cell::new(0));
            let instance = unsafe { GetModuleHandleW(null()) };
            let registration = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            assert_ne!(unsafe { RegisterClassW(&registration) }, 0);
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    class.as_ptr(),
                    class.as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    -10000,
                    -10000,
                    480,
                    240,
                    null_mut(),
                    null_mut(),
                    instance,
                    (&*paints as *const Cell<usize>).cast(),
                )
            };
            assert!(!hwnd.is_null());
            Self {
                hwnd,
                class,
                paints,
            }
        }
        fn rect(&self) -> [i32; 4] {
            let mut rect = RECT::default();
            assert_ne!(unsafe { GetWindowRect(self.hwnd, &mut rect) }, 0);
            [rect.left, rect.top, rect.right, rect.bottom]
        }
    }
    impl HasWindowHandle for Window {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let handle = Win32WindowHandle::new(NonZeroIsize::new(self.hwnd as isize).unwrap());
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
        }
    }
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.hwnd);
                UnregisterClassW(self.class.as_ptr(), GetModuleHandleW(null()));
            }
        }
    }

    #[test]
    fn sign_in_start_cloaks_the_window_until_it_is_uncloaked() {
        let (_temp, mut app, _ctx) = super::super::interaction_tests::fixture();
        let window = Window::new();
        let cloaked = |window: &Window| {
            let mut value = 0_i32;
            let result = unsafe {
                DwmGetWindowAttribute(
                    window.hwnd,
                    windows_sys::Win32::Graphics::Dwm::DWMWA_CLOAKED as u32,
                    (&mut value as *mut i32).cast(),
                    size_of::<i32>() as u32,
                )
            };
            assert!(result >= 0);
            value != 0
        };
        assert!(!cloaked(&window));
        app.start_in_tray(&window);
        assert!(app.hide_on_start && app.cloaked_start);
        assert!(cloaked(&window));
        set_cloaked(&window, false).unwrap();
        assert!(!cloaked(&window));
    }

    #[test]
    fn native_title_theme_applies_before_show_and_repaints_without_restore() {
        let (_temp, mut app, ctx) = super::super::interaction_tests::fixture();
        let window = Window::new();
        let original_rect = window.rect();
        app.configure_appearance(&ctx);
        app.sync_window_frame(&ctx, &window);
        assert!(!app.window_frame.unavailable);
        assert!(dark_mode(window.hwnd).unwrap());
        assert_eq!(unsafe { IsWindowVisible(window.hwnd) }, 0);

        unsafe { ShowWindow(window.hwnd, SW_SHOWNOACTIVATE) };
        for appearance in [
            super::super::Appearance::Light,
            super::super::Appearance::Dark,
        ] {
            let paints = window.paints.get();
            let mut preferences = app.interface;
            preferences.appearance = appearance;
            app.save_interface(&ctx, preferences);
            app.sync_window_frame(&ctx, &window);
            assert_eq!(
                dark_mode(window.hwnd).unwrap(),
                appearance == super::super::Appearance::Dark
            );
            assert!(
                window.paints.get() > paints,
                "Theme change must repaint the caption immediately"
            );
            let paints = window.paints.get();
            app.sync_window_frame(&ctx, &window);
            assert_eq!(
                window.paints.get(),
                paints,
                "Unchanged UI passes must not keep repainting"
            );
        }
        assert_eq!(window.rect(), original_rect);
        assert_ne!(unsafe { GetForegroundWindow() }, window.hwnd);

        // No late SetTheme command may undo the already-painted native theme.
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        assert!(output.viewport_output.values().all(|viewport| {
            !viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::SetTheme(_)))
        }));
    }

    #[test]
    fn native_title_theme_follows_system_and_recovers_external_theme_reset() {
        let (_temp, mut app, ctx) = super::super::interaction_tests::fixture();
        let window = Window::new();
        app.interface.appearance = super::super::Appearance::System;
        app.configure_appearance(&ctx);
        for theme in [egui::Theme::Light, egui::Theme::Dark, egui::Theme::Light] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    system_theme: Some(theme),
                    ..Default::default()
                },
                |_| {},
            );
            output.textures_delta.clear();
            app.sync_window_frame(&ctx, &window);
            assert_eq!(dark_mode(window.hwnd).unwrap(), theme == egui::Theme::Dark);
        }
        app.interface.appearance = super::super::Appearance::Dark;
        app.configure_appearance(&ctx);
        app.sync_window_frame(&ctx, &window);
        let light = 0_i32;
        assert_eq!(
            unsafe {
                DwmSetWindowAttribute(
                    window.hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                    (&light as *const i32).cast(),
                    size_of::<i32>() as u32,
                )
            },
            0
        );
        assert!(!dark_mode(window.hwnd).unwrap());
        app.sync_window_frame(&ctx, &window);
        assert!(dark_mode(window.hwnd).unwrap());
    }

    /// Lets the window thread process paint messages for `ms` milliseconds.
    fn pump(ms: u64) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while std::time::Instant::now() < deadline {
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }

    /// Luminance (0-255) of the caption as composed on screen, read in the middle of
    /// the caption, away from the title text and caption buttons. Panics if another
    /// window covers that point, since the reading would then describe that window.
    fn caption_luminance(hwnd: HWND) -> f32 {
        use windows_sys::Win32::{Foundation::POINT, Graphics::Gdi::*};
        let mut frame = RECT::default();
        let result = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                windows_sys::Win32::Graphics::Dwm::DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                (&mut frame as *mut RECT).cast(),
                size_of::<RECT>() as u32,
            )
        };
        assert!(result >= 0);
        let point = POINT {
            x: frame.left + (frame.right - frame.left) / 2,
            y: frame.top + 8,
        };
        assert_eq!(
            unsafe { WindowFromPoint(point) },
            hwnd,
            "Another window covers the test caption; cannot measure it"
        );
        let screen = unsafe { GetDC(null_mut()) };
        let color = unsafe { GetPixel(screen, point.x, point.y) };
        unsafe { ReleaseDC(null_mut(), screen) };
        let (r, g, b) = (color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF);
        0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32
    }

    // Reads composed screen pixels: a small blank window appears at the top-left corner
    // for about two seconds without taking focus. Run manually on Windows 10 and 11.
    #[test]
    #[ignore = "shows a window and reads composed screen pixels"]
    fn live_caption_pixels_follow_theme_changes() {
        let (_temp, mut app, ctx) = super::super::interaction_tests::fixture();
        let window = Window::new();
        unsafe {
            // A normal caption (not a tool window), never activated.
            SetWindowLongPtrW(window.hwnd, GWL_EXSTYLE, WS_EX_NOACTIVATE as isize);
            SetWindowPos(
                window.hwnd,
                HWND_TOPMOST,
                40,
                40,
                520,
                240,
                SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
            );
        }
        pump(300);
        let mut seen = Vec::new();
        for appearance in [
            super::super::Appearance::Dark,
            super::super::Appearance::Light,
            super::super::Appearance::Dark,
        ] {
            let mut preferences = app.interface;
            preferences.appearance = appearance;
            app.save_interface(&ctx, preferences);
            app.sync_window_frame(&ctx, &window);
            pump(400);
            seen.push((appearance, caption_luminance(window.hwnd)));
        }
        eprintln!("caption luminance: {seen:?}");
        let [(_, dark), (_, light), (_, dark_again)] = seen[..] else {
            unreachable!()
        };
        assert!(
            light > dark + 80.0,
            "Light theme left a dark caption: {seen:?}"
        );
        assert!(
            light > dark_again + 80.0,
            "Dark theme left a light caption: {seen:?}"
        );
    }

    #[test]
    fn native_title_theme_failure_restores_framework_handling() {
        struct Unavailable;
        impl HasWindowHandle for Unavailable {
            fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
                Err(HandleError::Unavailable)
            }
        }
        let (_temp, mut app, ctx) = super::super::interaction_tests::fixture();
        app.sync_window_frame(&ctx, &Unavailable);
        assert!(app.window_frame.unavailable);
        assert!(ctx.options(|options| options.sync_window_theme));
    }
}
