//! Folders dropped on the Dock icon, or opened with `open -a Cairn <dir>`,
//! reach the app as an "open documents" Apple Event. Each one becomes a tab
//! in that folder; at launch, they take the place of the first tab.
//!
//! `Info.plist` declares that Cairn opens folders (`CFBundleDocumentTypes`),
//! which is what lets the Dock accept the drop.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use eframe::egui;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSApplicationWillFinishLaunchingNotification;
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSInteger, NSNotification, NSNotificationCenter,
};

// Four-character codes from `AppleEvents.h`.
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

/// Folders received and not yet opened as tabs.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Wakes the window when a folder arrives while Cairn is running.
static WAKER: OnceLock<egui::Context> = OnceLock::new();

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Listener does
    // not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CairnOpenFolders"]
    struct Listener;

    impl Listener {
        /// AppKit installs its own "open documents" handler while launching
        /// and dispatches the launch event right after this notification:
        /// replacing the handler here is what Apple documents.
        #[unsafe(method(willFinishLaunching:))]
        fn will_finish_launching(&self, _: &NSNotification) {
            let manager = NSAppleEventManager::sharedAppleEventManager();
            // SAFETY: the selector is `handle_open`'s, and the listener
            // lives for the whole process (see `listen`).
            let _: () = unsafe {
                msg_send![
                    &manager,
                    setEventHandler: self,
                    andSelector: sel!(handleOpen:withReply:),
                    forEventClass: CORE_EVENT_CLASS,
                    andEventID: OPEN_DOCUMENTS,
                ]
            };
        }

        #[unsafe(method(handleOpen:withReply:))]
        fn handle_open(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let folders = folders(event);
            if folders.is_empty() {
                return;
            }
            PENDING
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(folders);
            if let Some(ctx) = WAKER.get() {
                ctx.request_repaint();
            }
        }
    }
);

/// Starts listening. On the main thread, before the event loop starts.
pub fn listen() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: plain NSObject initializer.
    let listener: Retained<Listener> = unsafe { msg_send![Listener::alloc(mtm), init] };
    // SAFETY: the selector is `will_finish_launching`'s; the name is a
    // constant exported by AppKit.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &listener,
            sel!(willFinishLaunching:),
            Some(NSApplicationWillFinishLaunchingNotification),
            None,
        );
    }
    // Neither the notification center nor the Apple Event manager keep it
    // alive, and it is needed until the process exits.
    std::mem::forget(listener);
}

/// Where to wake the window from when a folder arrives.
pub fn set_waker(ctx: &egui::Context) {
    let _ = WAKER.set(ctx.clone());
}

/// The folders received since the last call.
pub fn take() -> Vec<String> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Paths of the folders in the event: its direct object is a list of file
/// URLs, or a single one.
fn folders(event: &NSAppleEventDescriptor) -> Vec<String> {
    // SAFETY: `paramDescriptorForKeyword:` takes an AEKeyword (a UInt32).
    let Some(direct): Option<Retained<NSAppleEventDescriptor>> =
        (unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] })
    else {
        return Vec::new();
    };
    let n = direct.numberOfItems();
    let items: Vec<Retained<NSAppleEventDescriptor>> = if n > 0 {
        // AppleEvent lists are 1-based.
        (1..=n)
            .filter_map(|i: NSInteger| unsafe { msg_send![&direct, descriptorAtIndex: i] })
            .collect()
    } else {
        vec![direct]
    };
    items
        .iter()
        .filter_map(|d| d.fileURLValue()?.path())
        .map(|p| p.to_string())
        .filter(|p| Path::new(p).is_dir())
        .collect()
}
