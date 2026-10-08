//! macOS-specific window tweaks (AppKit).

use objc2_app_kit::{NSColor, NSTitlebarSeparatorStyle, NSView, NSWindowButton};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Rounds the window corners: transparent background and the content view's
/// layer clipped with `cornerRadius`. The system shadow is recomputed to
/// follow the new shape.
pub fn round_corners(frame: &eframe::Frame, radius: f64) -> bool {
    let Ok(handle) = frame.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return false;
    };
    // SAFETY: same as in center_traffic_lights.
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    let Some(window) = view.window() else {
        return false;
    };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    // No line drawn by macOS under the title bar.
    window.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
    view.setWantsLayer(true);
    let Some(layer) = view.layer() else {
        return false;
    };
    layer.setCornerRadius(radius);
    layer.setMasksToBounds(true);
    window.invalidateShadow();
    true
}

/// Grows the title bar container to `height` points and centers the
/// close/minimize/zoom buttons in it (what Electron does with
/// `trafficLightPosition`). AppKit repositions them on resize, so this is
/// called every frame; it only touches the window if something changed.
/// Returns the right edge of the last button, in points.
pub fn center_traffic_lights(frame: &eframe::Frame, height: f32) -> Option<f64> {
    let handle = frame.window_handle().ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return None;
    };
    // SAFETY: eframe guarantees ns_view is a live NSView while the
    // window exists, and this runs on the main thread.
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    let window = view.window()?;
    let height = height as f64;
    let close = window.standardWindowButton(NSWindowButton::CloseButton)?;
    // button → NSTitlebarView → NSTitlebarContainerView
    let container = unsafe { close.superview()?.superview() }?;
    let win_h = window.frame().size.height;
    let c = container.frame();
    if (c.size.height - height).abs() > 0.5 || (c.origin.y - (win_h - height)).abs() > 0.5 {
        container.setFrame(NSRect::new(
            NSPoint::new(c.origin.x, win_h - height),
            NSSize::new(c.size.width, height),
        ));
    }
    let mut right = 0.0f64;
    for (i, kind) in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ]
    .into_iter()
    .enumerate()
    {
        let button = window.standardWindowButton(kind)?;
        let parent = unsafe { button.superview() }?;
        let f = button.frame();
        let parent_h = parent.frame().size.height;
        let x = 16.0 + i as f64 * 20.0;
        let y = if parent.isFlipped() {
            (height - f.size.height) / 2.0
        } else {
            parent_h - height + (height - f.size.height) / 2.0
        };
        if (f.origin.x - x).abs() > 0.5 || (f.origin.y - y).abs() > 0.5 {
            button.setFrameOrigin(NSPoint::new(x, y));
        }
        right = right.max(x + f.size.width);
    }
    Some(right)
}
