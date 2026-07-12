use image::RgbaImage;
use std::{ffi::c_void, mem::size_of, ptr};
use windows::{
    core::{BOOL, PCWSTR},
    Win32::{
        Foundation::{LPARAM, POINT, RECT},
        Graphics::Gdi::{
            BitBlt, CreateCompatibleDC, CreateDCW, CreateDIBSection, DeleteDC, DeleteObject,
            EnumDisplayMonitors, GdiFlush, GetDeviceCaps, GetMonitorInfoW, MonitorFromPoint,
            SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DESKTOPHORZRES, DIB_RGB_COLORS,
            HBITMAP, HDC, HGDIOBJ, HMONITOR, HORZRES, MONITORINFO, MONITORINFOEXW,
            MONITOR_DEFAULTTONULL, SRCCOPY,
        },
        UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
    },
};

/// A full 5K frame is about 14.7 megapixels. Keeping the ceiling just above
/// that protects the GDI and OCR allocation paths without restricting normal
/// single-display selections.
pub const MAX_CAPTURE_PIXELS: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub work_x: i32,
    pub work_y: i32,
    pub work_width: u32,
    pub work_height: u32,
    pub scale_factor: f32,
    pub is_primary: bool,
}

#[derive(Default)]
struct MonitorEnumeration {
    handles: Vec<HMONITOR>,
    allocation_failed: bool,
}

pub fn displays() -> Result<Vec<Display>, String> {
    let mut enumeration = MonitorEnumeration {
        handles: Vec::with_capacity(4),
        allocation_failed: false,
    };
    let state = LPARAM(ptr::from_mut(&mut enumeration) as isize);
    // SAFETY: `state` points to the live, uniquely borrowed `enumeration` value.
    // `EnumDisplayMonitors` invokes the callback synchronously and does not retain
    // the LPARAM after this call returns; null clipping/DC arguments are permitted.
    let succeeded = unsafe { EnumDisplayMonitors(None, None, Some(enumerate_monitor), state) };
    if !succeeded.as_bool() {
        return if enumeration.allocation_failed {
            Err("Cannot allocate the display list".to_owned())
        } else {
            Err(format!(
                "Cannot enumerate displays: {}",
                std::io::Error::last_os_error()
            ))
        };
    }

    enumeration
        .handles
        .into_iter()
        .map(display_from_monitor)
        .collect()
}

pub fn capture_rgba(x: i32, y: i32, width: u32, height: u32) -> Result<RgbaImage, String> {
    let geometry = CaptureGeometry::validate(x, y, width, height)?;
    validate_monitor_bounds(geometry)?;

    let screen_dc = ScreenDc::acquire()?;
    let surface = CaptureSurface::new(screen_dc.handle(), geometry.width, geometry.height)?;
    // SAFETY: both DC handles remain valid for the duration of the call, the
    // destination DC has `surface.bitmap` selected into it, and validation above
    // guarantees positive dimensions and source coordinates inside one monitor.
    unsafe {
        BitBlt(
            surface.dc,
            0,
            0,
            geometry.width,
            geometry.height,
            Some(screen_dc.handle()),
            geometry.x,
            geometry.y,
            SRCCOPY,
        )
    }
    .map_err(|error| format!("Cannot copy the OCR region from the desktop: {error}"))?;
    // SAFETY: `GdiFlush` has no pointer or handle arguments and merely completes
    // the calling thread's queued GDI operations before the DIB memory is read.
    if !unsafe { GdiFlush() }.as_bool() {
        return Err("Cannot synchronize the captured desktop pixels".to_owned());
    }

    // SAFETY: a successful 32-bpp `CreateDIBSection` returned `surface.bits`.
    // `geometry.bytes` is exactly width * height * 4 (checked without overflow),
    // and `surface` keeps the DIB allocated and selected for the slice lifetime.
    let source = unsafe { std::slice::from_raw_parts(surface.bits.cast::<u8>(), geometry.bytes) };
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(geometry.bytes)
        .map_err(|_| "The OCR capture is too large for available memory".to_owned())?;
    rgba.resize(geometry.bytes, 0);
    for (bgra, output) in source.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
        output[0] = bgra[2];
        output[1] = bgra[1];
        output[2] = bgra[0];
        output[3] = bgra[3];
    }

    RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "Captured desktop pixels have an invalid layout".to_owned())
}

unsafe extern "system" fn enumerate_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    state: LPARAM,
) -> BOOL {
    // SAFETY: `displays` passes a live, uniquely borrowed MonitorEnumeration
    // pointer as LPARAM and EnumDisplayMonitors invokes this callback
    // synchronously before that borrow ends.
    let Some(enumeration) = (unsafe { (state.0 as *mut MonitorEnumeration).as_mut() }) else {
        return BOOL(0);
    };
    if enumeration.handles.try_reserve(1).is_err() {
        enumeration.allocation_failed = true;
        return BOOL(0);
    }
    enumeration.handles.push(monitor);
    BOOL(1)
}

fn display_from_monitor(monitor: HMONITOR) -> Result<Display, String> {
    let info = monitor_info(monitor)?;
    let rect = info.monitorInfo.rcMonitor;
    let width = positive_extent(rect.left, rect.right, "width")?;
    let height = positive_extent(rect.top, rect.bottom, "height")?;
    let work = info.monitorInfo.rcWork;
    let work_width = positive_extent(work.left, work.right, "work area width")?;
    let work_height = positive_extent(work.top, work.bottom, "work area height")?;
    Ok(Display {
        id: legacy_display_id(&info.szDevice),
        x: rect.left,
        y: rect.top,
        width,
        height,
        work_x: work.left,
        work_y: work.top,
        work_width,
        work_height,
        scale_factor: display_scale_factor(monitor, &info.szDevice),
        is_primary: info.monitorInfo.dwFlags & 1 != 0,
    })
}

fn monitor_info(monitor: HMONITOR) -> Result<MONITORINFOEXW, String> {
    if monitor.is_invalid() {
        return Err("No display contains the OCR region".to_owned());
    }
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `monitor` was checked for validity, `info` is a live, writable,
    // correctly aligned MONITORINFOEXW whose `cbSize` identifies its full size;
    // casting to the required MONITORINFO prefix preserves the same allocation.
    let succeeded =
        unsafe { GetMonitorInfoW(monitor, ptr::from_mut(&mut info).cast::<MONITORINFO>()) };
    if !succeeded.as_bool() {
        return Err(format!(
            "Cannot read display geometry: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(info)
}

fn validate_monitor_bounds(geometry: CaptureGeometry) -> Result<(), String> {
    // SAFETY: `MonitorFromPoint` accepts every integer POINT value, takes no
    // borrowed pointers, and `MONITOR_DEFAULTTONULL` is a valid selection flag.
    let monitor = unsafe {
        MonitorFromPoint(
            POINT {
                x: geometry.x,
                y: geometry.y,
            },
            MONITOR_DEFAULTTONULL,
        )
    };
    let rect = monitor_info(monitor)?.monitorInfo.rcMonitor;
    let right = i64::from(geometry.x) + i64::from(geometry.width);
    let bottom = i64::from(geometry.y) + i64::from(geometry.height);
    if geometry.x < rect.left
        || geometry.y < rect.top
        || right > i64::from(rect.right)
        || bottom > i64::from(rect.bottom)
    {
        return Err("The OCR region must fit entirely inside one display".to_owned());
    }
    Ok(())
}

fn positive_extent(start: i32, end: i32, name: &str) -> Result<u32, String> {
    let extent = i64::from(end) - i64::from(start);
    u32::try_from(extent)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("Display {name} is invalid"))
}

fn display_scale_factor(monitor: HMONITOR, device: &[u16; 32]) -> f32 {
    let (mut dpi_x, mut dpi_y) = (0_u32, 0_u32);
    // SAFETY: `monitor` came from EnumDisplayMonitors, both DPI pointers are
    // valid writable u32 values for the synchronous call, and the effective
    // DPI selector is a documented MONITOR_DPI_TYPE value.
    if unsafe {
        GetDpiForMonitor(
            monitor,
            MDT_EFFECTIVE_DPI,
            ptr::from_mut(&mut dpi_x),
            ptr::from_mut(&mut dpi_y),
        )
    }
    .is_ok()
        && (48..=768).contains(&dpi_x)
        && (48..=768).contains(&dpi_y)
    {
        return dpi_x as f32 / 96.0;
    }

    // Keep a GDI fallback for restricted sessions where the monitor DPI API
    // is unavailable (for example, some remote-desktop transitions).
    // SAFETY: `device` is the fixed-size `szDevice` buffer populated by a
    // successful GetMonitorInfoW call, so its storage is live and contains the
    // NUL-terminated display-device name expected by CreateDCW for both strings.
    let dc = unsafe {
        CreateDCW(
            PCWSTR(device.as_ptr()),
            PCWSTR(device.as_ptr()),
            PCWSTR::null(),
            None,
        )
    };
    if dc.is_invalid() {
        return 1.0;
    }
    // SAFETY: `dc` is a valid display DC returned by CreateDCW and remains live.
    let logical = unsafe { GetDeviceCaps(Some(dc), HORZRES) };
    // SAFETY: `dc` is a valid display DC returned by CreateDCW and remains live.
    let physical = unsafe { GetDeviceCaps(Some(dc), DESKTOPHORZRES) };
    // SAFETY: this function owns the CreateDCW result, it is not selected into
    // another DC, and it is never used again after this matching DeleteDC call.
    unsafe {
        let _ = DeleteDC(dc);
    }
    if logical <= 0 || physical <= 0 {
        1.0
    } else {
        (physical as f32 / logical as f32).max(1.0)
    }
}

// Keep the IDs generated by display-info/fxhash 0.4/0.2 so existing saved OCR
// display selections survive the backend replacement. The input is UTF-8 and
// the legacy code hashes a byte slice, including its Hash length prefix.
fn legacy_display_id(device: &[u16; 32]) -> u32 {
    const SEED: u32 = 0x2722_0a95;

    fn word(hash: u32, value: u32) -> u32 {
        (hash.rotate_left(5) ^ value).wrapping_mul(SEED)
    }

    let end = device
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(device.len());
    let name = String::from_utf16_lossy(&device[..end]);
    let bytes = name.as_bytes();
    let mut hash = word(0, bytes.len() as u32);
    // FxHasher32::write_usize hashes both halves on 64-bit Windows.
    hash = word(hash, 0);
    let mut chunks = bytes.chunks_exact(4);
    for chunk in &mut chunks {
        hash = word(
            hash,
            u32::from_le_bytes(chunk.try_into().expect("four bytes")),
        );
    }
    for byte in chunks.remainder() {
        hash = word(hash, u32::from(*byte));
    }
    hash
}

#[derive(Clone, Copy)]
struct CaptureGeometry {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    bytes: usize,
}

impl CaptureGeometry {
    fn validate(x: i32, y: i32, width: u32, height: u32) -> Result<Self, String> {
        let width_i32 = i32::try_from(width)
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| "OCR capture width is invalid".to_owned())?;
        let height_i32 = i32::try_from(height)
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| "OCR capture height is invalid".to_owned())?;
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or_else(|| "OCR capture dimensions overflow".to_owned())?;
        if pixels > MAX_CAPTURE_PIXELS {
            return Err(format!(
                "OCR capture exceeds the {} megapixel safety limit",
                MAX_CAPTURE_PIXELS / 1024 / 1024
            ));
        }
        let bytes = usize::try_from(
            pixels
                .checked_mul(4)
                .ok_or_else(|| "OCR capture byte size overflows".to_owned())?,
        )
        .map_err(|_| "OCR capture byte size is unsupported".to_owned())?;
        Ok(Self {
            x,
            y,
            width: width_i32,
            height: height_i32,
            bytes,
        })
    }
}

struct ScreenDc(HDC);

impl ScreenDc {
    fn acquire() -> Result<Self, String> {
        // SAFETY: passing a null HWND requests the desktop DC and supplies no
        // borrowed pointer; a successful handle is paired with ReleaseDC in Drop.
        let dc = unsafe { windows::Win32::Graphics::Gdi::GetDC(None) };
        if dc.is_invalid() {
            Err("Cannot acquire the desktop device context".to_owned())
        } else {
            Ok(Self(dc))
        }
    }

    fn handle(&self) -> HDC {
        self.0
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the successful GetDC(None) result owned by this
        // value, and the same null HWND is used for its one matching release.
        unsafe {
            windows::Win32::Graphics::Gdi::ReleaseDC(None, self.0);
        }
    }
}

/// Owns the memory DC, its selected top-down DIB, and the prior selected GDI
/// object as one indivisible unit. This guarantees the bitmap is deselected
/// before it is deleted on every return and unwind path.
struct CaptureSurface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut c_void,
}

impl CaptureSurface {
    fn new(screen: HDC, width: i32, height: i32) -> Result<Self, String> {
        // SAFETY: `screen` is a live desktop DC owned by the caller; the returned
        // compatible memory DC is checked and then owned by this constructor.
        let dc = unsafe { CreateCompatibleDC(Some(screen)) };
        if dc.is_invalid() {
            return Err("Cannot create the OCR capture device context".to_owned());
        }

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits = ptr::null_mut();
        // SAFETY: `screen` is live; `info` is a fully initialized 32-bpp RGB DIB
        // descriptor with positive width and a representable negative height;
        // `bits` is a valid writable out-pointer for the mapped pixel address.
        let bitmap = match unsafe {
            CreateDIBSection(
                Some(screen),
                ptr::from_ref(&info),
                DIB_RGB_COLORS,
                ptr::from_mut(&mut bits),
                None,
                0,
            )
        } {
            Ok(bitmap) if !bitmap.is_invalid() && !bits.is_null() => bitmap,
            Ok(bitmap) => {
                if !bitmap.is_invalid() {
                    // SAFETY: this constructor owns the successfully created,
                    // not-yet-selected bitmap and will not use it after deletion.
                    unsafe {
                        let _ = DeleteObject(bitmap.into());
                    }
                }
                // SAFETY: `dc` is the valid unshared CreateCompatibleDC result;
                // no object was selected by this constructor before this cleanup.
                unsafe {
                    let _ = DeleteDC(dc);
                }
                return Err("Cannot allocate the OCR capture bitmap".to_owned());
            }
            Err(error) => {
                // SAFETY: `dc` is the valid unshared CreateCompatibleDC result;
                // DIB creation failed, so this constructor selected nothing into it.
                unsafe {
                    let _ = DeleteDC(dc);
                }
                return Err(format!("Cannot allocate the OCR capture bitmap: {error}"));
            }
        };
        // SAFETY: both `dc` and `bitmap` are valid handles owned here; the bitmap
        // is not selected elsewhere. The returned previous object is retained so
        // Drop can restore it before deleting the bitmap.
        let previous = unsafe { SelectObject(dc, bitmap.into()) };
        if previous.is_invalid() {
            // SAFETY: a failed SelectObject leaves the DC selection unchanged;
            // both handles are owned here, and neither is used after cleanup.
            unsafe {
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(dc);
            }
            return Err("Cannot select the OCR capture bitmap".to_owned());
        }
        Ok(Self {
            dc,
            bitmap,
            previous,
            bits,
        })
    }
}

impl Drop for CaptureSurface {
    fn drop(&mut self) {
        // SAFETY: `dc` is live, currently has `bitmap` selected, and `previous`
        // is the valid object returned when that bitmap was selected. Restoring it
        // makes the bitmap safe to delete on the normal path below.
        let restored = unsafe { SelectObject(self.dc, self.previous) };
        if restored.is_invalid() {
            // Deleting the DC first releases its object selections, allowing the
            // bitmap to be reclaimed even if restoring the original object failed.
            // SAFETY: both handles are exclusively owned here. Deleting the DC
            // first releases any selection, after which the bitmap can be deleted;
            // Drop never accesses either handle again.
            unsafe {
                let _ = DeleteDC(self.dc);
                let _ = DeleteObject(self.bitmap.into());
            }
        } else {
            // SAFETY: successful restoration deselected `bitmap`; this value owns
            // both handles, deletes each exactly once, and performs no later use.
            unsafe {
                let _ = DeleteObject(self.bitmap.into());
                let _ = DeleteDC(self.dc);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetGuiResources, GR_GDIOBJECTS};

    #[test]
    fn keeps_legacy_display_ids() {
        let mut display = [0_u16; 32];
        let name = "\\\\.\\DISPLAY1".encode_utf16().collect::<Vec<_>>();
        display[..name.len()].copy_from_slice(&name);
        assert_eq!(legacy_display_id(&display), 2_776_250_164);
    }

    #[test]
    fn rejects_oversized_capture_allocations() {
        assert!(CaptureGeometry::validate(0, 0, 5120, 2880).is_ok());
        assert!(CaptureGeometry::validate(0, 0, 8192, 4320).is_err());
        assert!(CaptureGeometry::validate(0, 0, 0, 100).is_err());
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop"]
    fn repeated_capture_does_not_leak_gdi_objects() {
        let display = displays()
            .expect("displays should enumerate")
            .into_iter()
            .next()
            .expect("an interactive display should exist");
        capture_rgba(display.x, display.y, 8, 8).expect("warm-up capture should succeed");
        // SAFETY: GetCurrentProcess takes no external pointer and returns the
        // always-valid pseudo-handle for this process; it must not be closed.
        let process = unsafe { GetCurrentProcess() };
        // SAFETY: `process` is this process's valid pseudo-handle and
        // `GR_GDIOBJECTS` requests the documented GDI-object count.
        let before = unsafe { GetGuiResources(process, GR_GDIOBJECTS) };
        for _ in 0..500 {
            std::hint::black_box(
                capture_rgba(display.x, display.y, 8, 8).expect("capture should succeed"),
            );
        }
        // SAFETY: `process` remains a valid pseudo-handle for this process and
        // `GR_GDIOBJECTS` is the same documented query used for the baseline.
        let after = unsafe { GetGuiResources(process, GR_GDIOBJECTS) };
        assert!(
            after <= before.saturating_add(1),
            "GDI object count grew from {before} to {after}"
        );
    }
}
