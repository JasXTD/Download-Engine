//! Floating drop box: a small always-on-top box that accepts links dragged from
//! browsers and other apps, also while the main window is minimized or in the tray.
//! Its window, OLE registration and menu live on one STA thread, like the tray. The
//! window procedure only queues messages; the thread loop handles them.
use download_engine::i18n::Language;
use eframe::egui;
use std::sync::LazyLock;

/// Box and icon sizes in pixels at 100% scaling.
pub const BOX_SIZE: i32 = 64;
const ICON_SIZE: f32 = 40.0;
const RADIUS: f32 = 14.0;
/// Distance from the screen edge for the first placement, at 100% scaling.
const MARGIN: i32 = 24;

/// A monitor's work area (without the taskbar) in physical screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl Area {
    fn overlap(&self, x: i32, y: i32, size: i32) -> i64 {
        let (x, y, size) = (x as i64, y as i64, size as i64);
        let width = (self.right as i64).min(x + size) - (self.left as i64).max(x);
        let height = (self.bottom as i64).min(y + size) - (self.top as i64).max(y);
        width.max(0) * height.max(0)
    }
    /// Squared distance from a point to this area; zero inside it.
    fn distance(&self, x: i64, y: i64) -> i128 {
        let dx = (self.left as i64 - x).max(x - self.right as i64).max(0) as i128;
        let dy = (self.top as i64 - y).max(y - self.bottom as i64).max(0) as i128;
        dx * dx + dy * dy
    }
    /// Moves a `size` box fully inside, or to the top-left corner when it is larger.
    fn clamp(&self, x: i32, y: i32, size: i32) -> [i32; 2] {
        [
            x.min(self.right - size).max(self.left),
            y.min(self.bottom - size).max(self.top),
        ]
    }
}

/// Top-left corner for a box of `size` pixels. A saved corner is kept fully on the
/// work area it overlaps most, or the nearest one when its monitor is gone. Without
/// one, the box starts near the right edge of the primary work area.
pub fn place(saved: Option<[i32; 2]>, size: i32, areas: &[Area], primary: usize) -> [i32; 2] {
    let Some(main) = areas.get(primary).or(areas.first()) else {
        return saved.unwrap_or_default();
    };
    let Some([x, y]) = saved else {
        let margin = size * MARGIN / BOX_SIZE;
        return main.clamp(
            main.right - size - margin,
            main.top + (main.bottom - main.top - size) / 3,
            size,
        );
    };
    let (cx, cy) = (x as i64 + size as i64 / 2, y as i64 + size as i64 / 2);
    let area = areas
        .iter()
        .max_by_key(|area| {
            (
                area.overlap(x, y, size),
                std::cmp::Reverse(area.distance(cx, cy)),
            )
        })
        .unwrap_or(main);
    area.clamp(x, y, size)
}

/// Box size in physical pixels for a monitor DPI (96 is 100%).
pub fn scaled(dpi: u32) -> i32 {
    ((BOX_SIZE as i64 * dpi.max(48) as i64 + 48) / 96) as i32
}

/// Box colors as RGB. The hover colors show while a dragged link is over the box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub fill: [u8; 3],
    pub border: [u8; 3],
    pub hover_fill: [u8; 3],
    pub hover_border: [u8; 3],
}

/// The app icon cropped to its visible pixels, premultiplied RGBA.
struct Icon {
    width: usize,
    height: usize,
    rgba: Vec<f32>,
}
static ICON: LazyLock<Icon> = LazyLock::new(|| {
    let image = crate::tray::icon();
    let (width, height) = (image.width as usize, image.height as usize);
    let visible = |x: usize, y: usize| image.rgba[(y * width + x) * 4 + 3] > 8;
    let columns: Vec<_> = (0..width)
        .filter(|&x| (0..height).any(|y| visible(x, y)))
        .collect();
    let rows: Vec<_> = (0..height)
        .filter(|&y| (0..width).any(|x| visible(x, y)))
        .collect();
    let (left, right) = (columns[0], columns[columns.len() - 1] + 1);
    let (top, bottom) = (rows[0], rows[rows.len() - 1] + 1);
    let mut rgba = Vec::with_capacity((right - left) * (bottom - top) * 4);
    for y in top..bottom {
        for x in left..right {
            let pixel = &image.rgba[(y * width + x) * 4..][..4];
            let alpha = pixel[3] as f32 / 255.0;
            rgba.extend([
                pixel[0] as f32 * alpha,
                pixel[1] as f32 * alpha,
                pixel[2] as f32 * alpha,
                alpha,
            ]);
        }
    }
    Icon {
        width: right - left,
        height: bottom - top,
        rgba,
    }
});

/// Source pixels and weights for each destination pixel when shrinking `from` to `to`.
fn weights(from: usize, to: usize) -> Vec<Vec<(usize, f32)>> {
    let step = from as f32 / to as f32;
    (0..to)
        .map(|i| {
            let (start, end) = (i as f32 * step, (i + 1) as f32 * step);
            (start.floor() as usize..(end.ceil() as usize).min(from))
                .map(|s| {
                    let covered = (end.min(s as f32 + 1.0) - start.max(s as f32)).max(0.0);
                    (s, covered / step)
                })
                .collect()
        })
        .collect()
}

/// Premultiplied BGRA pixels, top row first, for a `size`×`size` box.
pub fn paint(size: i32, look: &Look, hover: bool) -> Vec<u8> {
    let size = size.max(1) as usize;
    let scale = size as f32 / BOX_SIZE as f32;
    let (fill, border, line) = if hover {
        (look.hover_fill, look.hover_border, 2.0 * scale)
    } else {
        (look.fill, look.border, scale.max(1.0))
    };
    let half = size as f32 / 2.0;
    let radius = (RADIUS * scale).min(half);
    // Premultiplied RGBA while composing.
    let mut pixels = vec![[0.0f32; 4]; size * size];
    for y in 0..size {
        for x in 0..size {
            // Signed distance from the pixel center to the rounded square's edge.
            let qx = (x as f32 + 0.5 - half).abs() - (half - radius);
            let qy = (y as f32 + 0.5 - half).abs() - (half - radius);
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            let distance = outside + qx.max(qy).min(0.0) - radius;
            let alpha = (0.5 - distance).clamp(0.0, 1.0);
            let inner = (0.5 - (distance + line)).clamp(0.0, 1.0);
            let mut pixel = [0.0; 4];
            for c in 0..3 {
                let value = border[c] as f32 * (1.0 - inner) + fill[c] as f32 * inner;
                pixel[c] = value * alpha;
            }
            pixel[3] = alpha;
            pixels[y * size + x] = pixel;
        }
    }
    let icon = &*ICON;
    let target = (ICON_SIZE * scale).round().max(1.0);
    let ratio = target / icon.width.max(icon.height) as f32;
    let (width, height) = (
        ((icon.width as f32 * ratio).round() as usize).clamp(1, size),
        ((icon.height as f32 * ratio).round() as usize).clamp(1, size),
    );
    let (columns, rows) = (weights(icon.width, width), weights(icon.height, height));
    let (left, top) = ((size - width) / 2, (size - height) / 2);
    for (dy, row) in rows.iter().enumerate() {
        for (dx, column) in columns.iter().enumerate() {
            let mut sample = [0.0f32; 4];
            for &(sy, wy) in row {
                for &(sx, wx) in column {
                    let source = &icon.rgba[(sy * icon.width + sx) * 4..][..4];
                    for c in 0..4 {
                        sample[c] += source[c] * wx * wy;
                    }
                }
            }
            let pixel = &mut pixels[(top + dy) * size + left + dx];
            for c in 0..4 {
                pixel[c] = sample[c] + pixel[c] * (1.0 - sample[3]);
            }
        }
    }
    pixels
        .into_iter()
        .flat_map(|[r, g, b, a]| {
            let byte = |value: f32| value.round().clamp(0.0, 255.0) as u8;
            // Premultiplied color never exceeds alpha, also after rounding.
            let alpha = byte(a * 255.0);
            [
                byte(b).min(alpha),
                byte(g).min(alpha),
                byte(r).min(alpha),
                alpha,
            ]
        })
        .collect()
}

/// What the user did with the box; dropped links arrive through `DropTarget::drops`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Dragged to a new top-left corner, in physical screen pixels.
    Moved([i32; 2]),
    Open,
    Hide,
    Failed(String),
}

#[cfg(windows)]
pub use native::DropTarget;

#[cfg(windows)]
mod native {
    use super::*;
    use anyhow::{Context as _, Result, ensure};
    use download_engine::i18n::{self, tr};
    use std::{
        mem::size_of,
        ptr::{null, null_mut},
        sync::mpsc,
        thread,
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
            CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
            EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST,
            MONITORINFO, MonitorFromPoint, SelectObject,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI},
            Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture},
            WindowsAndMessaging::*,
        },
    };

    const CLASS: &str = "IDM_EX.DropTarget.v1";
    const WAKE: u32 = WM_APP + 1;
    const HOVER: u32 = WM_APP + 2;
    const OPEN: usize = 1;
    const HIDE: usize = 2;

    enum Command {
        Look(Look),
        Language(Language),
        Stop,
    }
    /// Message values only: pointers in WPARAM or LPARAM are invalid once queued.
    struct Callback {
        message: u32,
        w: usize,
        l: isize,
    }
    struct WindowContext {
        callbacks: mpsc::Sender<Callback>,
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    // The context is immutable and stays boxed until after DestroyWindow. Queuing
    // avoids Rust borrows across reentrant Win32 calls such as SetWindowPos.
    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if message == WM_NCCREATE {
                let create = &*(l as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowContext;
            if !pointer.is_null() {
                let _ = (*pointer).callbacks.send(Callback { message, w, l });
            }
            if message == WM_NCDESTROY {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            match message {
                // Clicking or dragging the box never takes focus from the user's app.
                WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
                // Only Download Engine closes the box, by stopping its thread.
                WM_CLOSE => 0,
                _ => DefWindowProcW(hwnd, message, w, l),
            }
        }
    }

    unsafe extern "system" fn monitor(
        handle: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> windows_sys::core::BOOL {
        unsafe {
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(handle, &mut info) != 0 {
                let areas = &mut *(data as *mut (Vec<Area>, usize));
                if info.dwFlags & MONITORINFOF_PRIMARY != 0 {
                    areas.1 = areas.0.len();
                }
                let work = info.rcWork;
                areas.0.push(Area {
                    left: work.left,
                    top: work.top,
                    right: work.right,
                    bottom: work.bottom,
                });
            }
            1
        }
    }
    /// Work areas of the connected monitors and the primary one's index.
    fn work_areas() -> (Vec<Area>, usize) {
        let mut areas = (Vec::new(), 0usize);
        unsafe {
            EnumDisplayMonitors(
                null_mut(),
                null(),
                Some(monitor),
                &mut areas as *mut (Vec<Area>, usize) as LPARAM,
            );
        }
        areas
    }
    fn dpi_at(point: [i32; 2]) -> u32 {
        let monitor = unsafe {
            MonitorFromPoint(
                POINT {
                    x: point[0],
                    y: point[1],
                },
                MONITOR_DEFAULTTONEAREST,
            )
        };
        let (mut x, mut y) = (0, 0);
        let result = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) };
        if result >= 0 && x > 0 { x } else { 96 }
    }
    fn cursor() -> POINT {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) };
        point
    }

    struct Drag {
        cursor: POINT,
        origin: [i32; 2],
        moving: bool,
    }
    struct Window {
        hwnd: HWND,
        _context: Box<WindowContext>,
        look: Look,
        language: Language,
        dpi: u32,
        position: [i32; 2],
        hover: bool,
        drag: Option<Drag>,
    }
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe { DestroyWindow(self.hwnd) };
        }
    }
    impl Window {
        fn new(
            saved: Option<[i32; 2]>,
            look: Look,
            language: Language,
            callbacks: mpsc::Sender<Callback>,
        ) -> Result<Self> {
            let context = Box::new(WindowContext { callbacks });
            let class = wide(CLASS);
            let instance = unsafe { GetModuleHandleW(null()) };
            ensure!(
                !instance.is_null(),
                "Cannot load the floating drop box module"
            );
            let registration = WNDCLASSW {
                style: CS_DBLCLKS,
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            if unsafe { RegisterClassW(&registration) } == 0 {
                ensure!(
                    std::io::Error::last_os_error().raw_os_error() == Some(1410),
                    "Cannot register the floating drop box window"
                );
            }
            // Estimate the DPI where the box will appear, then size it for that monitor.
            let (areas, primary) = work_areas();
            let estimate = place(saved, BOX_SIZE, &areas, primary);
            let dpi = dpi_at([estimate[0] + BOX_SIZE / 2, estimate[1] + BOX_SIZE / 2]);
            let size = scaled(dpi);
            let position = place(saved, size, &areas, primary);
            let title = {
                let _language = i18n::scope(language);
                wide(tr("Download Engine floating drop box"))
            };
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    class.as_ptr(),
                    title.as_ptr(),
                    WS_POPUP,
                    position[0],
                    position[1],
                    size,
                    size,
                    null_mut(),
                    null_mut(),
                    instance,
                    context.as_ref() as *const _ as *const _,
                )
            };
            ensure!(
                !hwnd.is_null(),
                "Cannot create the floating drop box window"
            );
            let mut window = Self {
                hwnd,
                _context: context,
                look,
                language,
                dpi,
                position,
                hover: false,
                drag: None,
            };
            window.refresh()?;
            Ok(window)
        }
        fn size(&self) -> i32 {
            scaled(self.dpi)
        }
        /// Follows the monitor's DPI and keeps the box on a visible work area.
        fn refresh(&mut self) -> Result<()> {
            let dpi = unsafe { GetDpiForWindow(self.hwnd) };
            if dpi != 0 {
                self.dpi = dpi;
            }
            let (areas, primary) = work_areas();
            self.position = place(Some(self.position), self.size(), &areas, primary);
            self.paint()
        }
        fn paint(&self) -> Result<()> {
            let size = self.size();
            let pixels = paint(size, &self.look, self.hover);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size,
                    // Negative height: rows run top to bottom, like `paint`.
                    biHeight: -size,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                ensure!(!dc.is_null(), "Cannot draw the floating drop box");
                let mut bits = null_mut();
                let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                if bitmap.is_null() || bits.is_null() {
                    DeleteDC(dc);
                    anyhow::bail!("Cannot draw the floating drop box");
                }
                std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
                let previous = SelectObject(dc, bitmap);
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let updated = UpdateLayeredWindow(
                    self.hwnd,
                    null_mut(),
                    &POINT {
                        x: self.position[0],
                        y: self.position[1],
                    },
                    &SIZE { cx: size, cy: size },
                    dc,
                    &POINT::default(),
                    0,
                    &blend,
                    ULW_ALPHA,
                );
                SelectObject(dc, previous);
                DeleteObject(bitmap);
                DeleteDC(dc);
                ensure!(updated != 0, "Cannot draw the floating drop box");
            }
            Ok(())
        }
        fn move_to(&mut self, position: [i32; 2]) -> Result<()> {
            let (areas, primary) = work_areas();
            self.position = place(Some(position), self.size(), &areas, primary);
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    self.position[0],
                    self.position[1],
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            // Crossing to a monitor with another scale changes the box size.
            if unsafe { GetDpiForWindow(self.hwnd) } != self.dpi {
                self.refresh()?;
            }
            Ok(())
        }
        fn set_language(&mut self, language: Language) {
            self.language = language;
            let _language = i18n::scope(language);
            unsafe {
                SetWindowTextW(
                    self.hwnd,
                    wide(tr("Download Engine floating drop box")).as_ptr(),
                )
            };
        }
        fn menu(&self, at: POINT) -> Option<Event> {
            let _language = i18n::scope(self.language);
            unsafe {
                let menu = CreatePopupMenu();
                if menu.is_null() {
                    return None;
                }
                AppendMenuW(
                    menu,
                    MF_STRING,
                    OPEN,
                    wide(tr("Open Download Engine")).as_ptr(),
                );
                AppendMenuW(menu, MF_SEPARATOR, 0, null());
                AppendMenuW(
                    menu,
                    MF_STRING,
                    HIDE,
                    wide(tr("Hide floating drop box")).as_ptr(),
                );
                // A menu closes on outside clicks only while its owner is in front.
                let previous = GetForegroundWindow();
                SetForegroundWindow(self.hwnd);
                let id = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                    at.x,
                    at.y,
                    0,
                    self.hwnd,
                    null(),
                ) as usize;
                PostMessageW(self.hwnd, WM_NULL, 0, 0);
                DestroyMenu(menu);
                // Return focus to the app the user was in unless Download Engine opens.
                if id != OPEN && !previous.is_null() && previous != self.hwnd {
                    SetForegroundWindow(previous);
                }
                match id {
                    OPEN => Some(Event::Open),
                    HIDE => Some(Event::Hide),
                    _ => None,
                }
            }
        }
        fn handle(&mut self, callback: Callback) -> Result<Option<Event>> {
            match callback.message {
                WM_LBUTTONDOWN => {
                    self.drag = Some(Drag {
                        cursor: cursor(),
                        origin: self.position,
                        moving: false,
                    });
                    unsafe { SetCapture(self.hwnd) };
                }
                WM_MOUSEMOVE => {
                    if let Some(drag) = &mut self.drag {
                        let now = cursor();
                        let (dx, dy) = (now.x - drag.cursor.x, now.y - drag.cursor.y);
                        let threshold =
                            unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
                        drag.moving |= dx.abs() >= threshold.0 || dy.abs() >= threshold.1;
                        if drag.moving {
                            let origin = drag.origin;
                            self.move_to([origin[0] + dx, origin[1] + dy])?;
                        }
                    }
                }
                WM_LBUTTONUP | WM_CAPTURECHANGED => {
                    if let Some(drag) = self.drag.take() {
                        if unsafe { GetCapture() } == self.hwnd {
                            unsafe { ReleaseCapture() };
                        }
                        if drag.moving {
                            return Ok(Some(Event::Moved(self.position)));
                        }
                    }
                }
                WM_LBUTTONDBLCLK => return Ok(Some(Event::Open)),
                WM_CONTEXTMENU => {
                    // A right-click reports its screen position; a keyboard menu (-1)
                    // opens below the box.
                    let at = if callback.l == -1 {
                        POINT {
                            x: self.position[0],
                            y: self.position[1] + self.size(),
                        }
                    } else {
                        POINT {
                            x: callback.l as u16 as i16 as i32,
                            y: (callback.l >> 16) as u16 as i16 as i32,
                        }
                    };
                    return Ok(self.menu(at));
                }
                HOVER => {
                    let hover = callback.w != 0;
                    if hover != self.hover {
                        self.hover = hover;
                        self.paint()?;
                    }
                }
                WM_DPICHANGED | WM_DISPLAYCHANGE => self.refresh()?,
                WM_SETTINGCHANGE if callback.w == SPI_SETWORKAREA as usize => self.refresh()?,
                _ => {}
            }
            Ok(None)
        }
    }

    /// The running floating drop box. Dropping it closes the window, revokes its OLE
    /// registration and ends its thread.
    pub struct DropTarget {
        hwnd: usize,
        commands: mpsc::Sender<Command>,
        events: mpsc::Receiver<Event>,
        /// Validated link text, one URL per line.
        pub drops: mpsc::Receiver<String>,
        thread: Option<thread::JoinHandle<()>>,
    }
    impl DropTarget {
        pub fn new(
            ctx: egui::Context,
            saved: Option<[i32; 2]>,
            look: Look,
            language: Language,
        ) -> Result<Self> {
            Self::start(ctx, saved, look, language, true)
        }
        fn start(
            ctx: egui::Context,
            saved: Option<[i32; 2]>,
            look: Look,
            language: Language,
            show: bool,
        ) -> Result<Self> {
            let (commands, rx) = mpsc::channel();
            let (events_tx, events) = mpsc::channel();
            let (drops_tx, drops) = mpsc::sync_channel(crate::url_drop::QUEUE);
            let (ready_tx, ready) = mpsc::sync_channel(1);
            let thread = thread::Builder::new()
                .name("idm-ex-drop-target".into())
                .spawn(move || {
                    let emit = |event| {
                        let _ = events_tx.send(event);
                        ctx.request_repaint();
                    };
                    let (callbacks_tx, callbacks) = mpsc::channel();
                    let mut window = match Window::new(saved, look, language, callbacks_tx) {
                        Ok(window) => window,
                        Err(error) => {
                            let _ = ready_tx.send(Err(format!("{error:#}")));
                            return;
                        }
                    };
                    let hwnd = window.hwnd as usize;
                    // Hover changes are posted so drag callbacks never repaint reentrantly.
                    let hover: Box<dyn Fn(bool)> = Box::new(move |accepted| unsafe {
                        PostMessageW(hwnd as HWND, HOVER, usize::from(accepted), 0);
                    });
                    let registration = match crate::url_drop::Guard::new(
                        hwnd as isize,
                        ctx.clone(),
                        drops_tx,
                        Some(hover),
                    ) {
                        Ok(registration) => registration,
                        Err(error) => {
                            let _ = ready_tx.send(Err(format!("{error:#}")));
                            return;
                        }
                    };
                    if show {
                        unsafe { ShowWindow(window.hwnd, SW_SHOWNOACTIVATE) };
                    }
                    if ready_tx.send(Ok(hwnd)).is_err() {
                        return;
                    }
                    loop {
                        let mut message = MSG::default();
                        if unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } <= 0 {
                            emit(Event::Failed(
                                "The floating drop box window stopped unexpectedly".into(),
                            ));
                            break;
                        }
                        unsafe {
                            TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                        let mut failure = None;
                        for callback in callbacks.try_iter() {
                            match window.handle(callback) {
                                Ok(Some(event)) => emit(event),
                                Ok(None) => {}
                                Err(error) => failure = Some(format!("{error:#}")),
                            }
                        }
                        if let Some(error) = failure {
                            emit(Event::Failed(error));
                            break;
                        }
                        // Read commands last: a menu loop may have consumed the wake message.
                        let mut stopping = false;
                        for command in rx.try_iter() {
                            match command {
                                Command::Look(look) => {
                                    window.look = look;
                                    if let Err(error) = window.paint() {
                                        failure = Some(format!("{error:#}"));
                                    }
                                }
                                Command::Language(language) => window.set_language(language),
                                Command::Stop => stopping = true,
                            }
                        }
                        if let Some(error) = failure {
                            emit(Event::Failed(error));
                            break;
                        }
                        if stopping {
                            break;
                        }
                    }
                    // Revoke OLE before destroying the window it is registered on.
                    drop(registration);
                    drop(window);
                })?;
            match ready
                .recv()
                .context("Floating drop box initialization stopped")?
            {
                Ok(hwnd) => Ok(Self {
                    hwnd,
                    commands,
                    events,
                    drops,
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
                .context("The floating drop box is unavailable")?;
            ensure!(
                unsafe { PostMessageW(self.hwnd as HWND, WAKE, 0, 0) } != 0,
                "Cannot wake the floating drop box"
            );
            Ok(())
        }
        pub fn events(&self) -> Vec<Event> {
            self.events.try_iter().collect()
        }
        pub fn set_look(&self, look: Look) -> Result<()> {
            self.send(Command::Look(look))
        }
        pub fn set_language(&self, language: Language) -> Result<()> {
            self.send(Command::Language(language))
        }
    }
    impl Drop for DropTarget {
        fn drop(&mut self) {
            let _ = self.commands.send(Command::Stop);
            unsafe {
                // Close an open menu, then wake the loop to read Stop.
                PostMessageW(self.hwnd as HWND, WM_CANCELMODE, 0, 0);
                PostMessageW(self.hwnd as HWND, WAKE, 0, 0);
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn hidden_drop_target_is_a_no_activate_tool_window_registered_for_ole_until_dropped() {
            let look = Look {
                fill: [38, 42, 45],
                border: [59, 66, 72],
                hover_fill: [36, 54, 71],
                hover_border: [125, 182, 232],
            };
            // A corner far outside every monitor must come back onto a work area.
            let target = DropTarget::start(
                egui::Context::default(),
                Some([-1_000_000, -1_000_000]),
                look,
                Language::English,
                false,
            )
            .unwrap();
            let hwnd = target.hwnd as HWND;
            let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
            for flag in [
                WS_EX_LAYERED,
                WS_EX_TOPMOST,
                WS_EX_TOOLWINDOW,
                WS_EX_NOACTIVATE,
            ] {
                assert_ne!(style & flag, 0, "{flag:#x}");
            }
            assert_eq!(unsafe { IsWindowVisible(hwnd) }, 0, "Tests never show it");
            // OLE keeps the registered drop target in this window property.
            assert!(!unsafe { GetPropW(hwnd, wide("OleDropTargetInterface").as_ptr()) }.is_null());
            let mut rect = RECT::default();
            assert_ne!(unsafe { GetWindowRect(hwnd, &mut rect) }, 0);
            let (areas, _) = work_areas();
            assert!(areas.iter().any(|area| rect.left >= area.left
                && rect.top >= area.top
                && rect.right <= area.right
                && rect.bottom <= area.bottom));
            assert_eq!(rect.right - rect.left, rect.bottom - rect.top);
            target.set_language(Language::German).unwrap();
            target.set_look(look).unwrap();
            assert!(target.events().is_empty());
            drop(target);
            assert_eq!(unsafe { IsWindow(hwnd) }, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREAS: [Area; 2] = [
        Area {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        },
        // A second monitor left of the primary one, with negative coordinates.
        Area {
            left: -1280,
            top: 200,
            right: 0,
            bottom: 1184,
        },
    ];

    #[test]
    fn first_placement_is_near_the_right_edge_of_the_primary_work_area() {
        assert_eq!(
            place(None, 64, &AREAS, 0),
            [1920 - 64 - 24, (1040 - 64) / 3]
        );
        // At 150% the size and margin both scale.
        assert_eq!(
            place(None, 96, &AREAS, 0),
            [1920 - 96 - 36, (1040 - 96) / 3]
        );
        assert_eq!(place(None, 64, &AREAS, 1)[0], -64 - 24);
        assert_eq!(place(Some([5, 6]), 64, &[], 0), [5, 6]);
    }

    #[test]
    fn saved_positions_stay_fully_on_a_visible_work_area() {
        // Fully visible: unchanged, including on a monitor with negative coordinates.
        assert_eq!(place(Some([100, 200]), 64, &AREAS, 0), [100, 200]);
        assert_eq!(place(Some([-600, 500]), 64, &AREAS, 0), [-600, 500]);
        // Partly below the taskbar or past the right edge: pulled back inside.
        assert_eq!(
            place(Some([1900, 1030]), 64, &AREAS, 0),
            [1920 - 64, 1040 - 64]
        );
        // Straddling two monitors: kept on the one it overlaps most.
        assert_eq!(place(Some([-40, 300]), 64, &AREAS, 0), [-64, 300]);
        assert_eq!(place(Some([-20, 300]), 64, &AREAS, 0), [0, 300]);
        assert_eq!(place(Some([-10, 300]), 64, &AREAS, 0), [0, 300]);
        // Above the second monitor's top edge.
        assert_eq!(place(Some([-600, 0]), 64, &AREAS, 0), [-600, 200]);
        // A disconnected monitor far away: moved to the nearest remaining one.
        assert_eq!(place(Some([4000, 500]), 64, &AREAS, 0), [1920 - 64, 500]);
        assert_eq!(place(Some([-5000, 900]), 64, &AREAS, 0), [-1280, 900]);
        assert_eq!(
            place(Some([i32::MAX, i32::MIN]), 64, &AREAS, 0),
            [1920 - 64, 0]
        );
        // A work area smaller than the box keeps its top-left corner visible.
        let tiny = [Area {
            left: 10,
            top: 20,
            right: 50,
            bottom: 60,
        }];
        assert_eq!(place(Some([0, 0]), 64, &tiny, 0), [10, 20]);
    }

    #[test]
    fn box_scales_with_monitor_dpi() {
        assert_eq!(scaled(96), 64);
        assert_eq!(scaled(120), 80);
        assert_eq!(scaled(144), 96);
        assert_eq!(scaled(192), 128);
        assert_eq!(scaled(0), 32);
    }

    #[test]
    fn painted_box_has_transparent_corners_a_theme_border_and_the_app_icon() {
        let look = Look {
            fill: [255, 255, 255],
            border: [201, 208, 215],
            hover_fill: [237, 244, 252],
            hover_border: [44, 114, 180],
        };
        let at = |pixels: &[u8], size: usize, x: usize, y: usize| -> [u8; 4] {
            pixels[(y * size + x) * 4..][..4].try_into().unwrap()
        };
        for (size, hover) in [(64, false), (64, true), (96, false), (128, true)] {
            let pixels = paint(size as i32, &look, hover);
            assert_eq!(pixels.len(), size * size * 4);
            // Rounded corners are fully transparent, so clicks and drops pass through.
            assert_eq!(at(&pixels, size, 0, 0), [0, 0, 0, 0]);
            assert_eq!(at(&pixels, size, size - 1, size - 1), [0, 0, 0, 0]);
            // The middle of the top edge is the opaque border color (BGRA).
            let [r, g, b] = if hover {
                look.hover_border
            } else {
                look.border
            };
            assert_eq!(at(&pixels, size, size / 2, 0), [b, g, r, 255]);
            // Inside the border, beside the icon, is the fill.
            let [r, g, b] = if hover { look.hover_fill } else { look.fill };
            assert_eq!(at(&pixels, size, size / 2, size / 16 + 2), [b, g, r, 255]);
            // The icon's blue square fills the middle, left of its white arrow.
            let [b, g, r, a] = at(&pixels, size, size / 2 - size * 12 / 64, size / 2);
            assert_eq!(a, 255);
            assert!(b > 200 && r < 100 && g > 80, "{:?}", [r, g, b]);
            // Premultiplied: no channel exceeds alpha.
            assert!(
                pixels
                    .chunks(4)
                    .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
            );
        }
    }
}
