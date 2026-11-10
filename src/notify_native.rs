//! Desktop notifications through `UNUserNotificationCenter` (macOS, from the app bundle only:
//! the framework refuses processes without a bundle identifier). Clicking a notification opens
//! the pane it is about (FR-N4); the pane id travels in the request identifier.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

use eframe::egui;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use parking_lot::Mutex;

/// The channel clicked panes go to, and the context to wake.
type ClickSink = Mutex<Option<(Sender<u64>, egui::Context)>>;

/// Where clicked panes go.
static CLICKS: OnceLock<ClickSink> = OnceLock::new();
static DELEGATE: OnceLock<Retained<NotificationDelegate>> = OnceLock::new();

fn pane_of(identifier: &str) -> Option<u64> {
    identifier
        .strip_prefix("tc-pane-")?
        .split('-')
        .next()?
        .parse()
        .ok()
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "TernaryCodeNotificationDelegate"]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            let id = response.notification().request().identifier().to_string();
            if let Some(pane) = pane_of(&id)
                && let Some(slot) = CLICKS.get()
                && let Some((tx, ctx)) = slot.lock().as_ref()
            {
                let _ = tx.send(pane);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
            completion.call(());
        }

        // Shown even while TernaryCode is the active app (the pane may be in another project).
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }
    }
);

impl NotificationDelegate {
    fn new() -> Retained<Self> {
        // SAFETY: plain NSObject subclass without instance variables.
        unsafe { msg_send![Self::alloc(), init] }
    }
}

/// Sets up native notifications when running from the app bundle. Returns false otherwise (the
/// caller falls back to `osascript`).
pub fn init(clicks: Sender<u64>, ctx: &egui::Context) -> bool {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return false;
    }
    let _ = CLICKS.set(Mutex::new(Some((clicks, ctx.clone()))));
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = DELEGATE.get_or_init(NotificationDelegate::new);
    center.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
    let done = block2::RcBlock::new(|_granted: Bool, _error: *mut NSError| {});
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge,
        &done,
    );
    true
}

/// Posts a notification about `pane`.
pub fn post(title: &str, subtitle: &str, body: &str, pane: u64) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setSubtitle(&NSString::from_str(subtitle));
    content.setBody(&NSString::from_str(body));
    let id = format!("tc-pane-{pane}-{}", COUNTER.fetch_add(1, Ordering::Relaxed));
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id), &content, None);
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, None);
}

#[cfg(test)]
mod tests {
    #[test]
    fn pane_ids_round_trip_through_identifiers() {
        assert_eq!(super::pane_of("tc-pane-42-7"), Some(42));
        assert_eq!(super::pane_of("other-1"), None);
    }
}
