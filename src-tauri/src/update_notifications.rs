use crate::model::Result;
use semver::Version;
use tauri::AppHandle;

const BODY: &str =
    "Öffne JunaDesk oder das Hintergrundmenü, um die neue Version auf GitHub anzusehen.";
const PERMISSION_ERROR: &str =
    "Die Berechtigung für Systembenachrichtigungen konnte nicht abgefragt werden.";

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use block2::{DynBlock, RcBlock};
    use objc2::{
        define_class, msg_send,
        rc::Retained,
        runtime::{Bool, ProtocolObject},
        ClassType,
    };
    use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
        UNNotification, UNNotificationPresentationOptions, UNNotificationRequest,
        UNNotificationSettings, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };
    use std::{cell::RefCell, ptr::NonNull, sync::Mutex, time::Duration};
    use tokio::sync::oneshot;

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "JunaDeskUpdateNotificationDelegate"]
        struct NotificationDelegate;

        unsafe impl NSObjectProtocol for NotificationDelegate {}

        unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                completion.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
            }
        }
    );

    thread_local! {
        // UNUserNotificationCenter references its delegate weakly.
        static DELEGATE: RefCell<Option<Retained<NotificationDelegate>>> = const { RefCell::new(None) };
    }

    fn has_app_bundle() -> bool {
        !tauri::is_dev() && NSBundle::mainBundle().bundleIdentifier().is_some()
    }

    pub async fn request(app: &AppHandle) -> Result<()> {
        if !has_app_bundle() {
            return Err(
                "Systembenachrichtigungen sind in der installierten JunaDesk-App verfügbar.".into(),
            );
        }
        let (send, receive) = oneshot::channel();
        app.run_on_main_thread(move || {
            let send = Mutex::new(Some(send));
            let callback = RcBlock::new(move |granted: Bool, error: *mut NSError| {
                let result = if !error.is_null() {
                    Err(PERMISSION_ERROR.into())
                } else if granted.as_bool() {
                    Ok(())
                } else {
                    Err("Erlaube JunaDesk Mitteilungen in den macOS-Systemeinstellungen.".into())
                };
                if let Some(send) = send.lock().unwrap().take() {
                    let _ = send.send(result);
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .requestAuthorizationWithOptions_completionHandler(
                    UNAuthorizationOptions::Alert,
                    &callback,
                );
        })
        .map_err(|_| PERMISSION_ERROR.to_owned())?;
        receive.await.map_err(|_| PERMISSION_ERROR.to_owned())?
    }

    pub async fn allowed(app: &AppHandle) -> bool {
        if !has_app_bundle() {
            return false;
        }
        let (send, receive) = oneshot::channel();
        if app
            .run_on_main_thread(move || {
                let send = Mutex::new(Some(send));
                let callback = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
                    // UserNotifications keeps the settings object alive during the callback.
                    let status = unsafe { settings.as_ref() }.authorizationStatus();
                    if let Some(send) = send.lock().unwrap().take() {
                        let _ = send.send(status == UNAuthorizationStatus::Authorized);
                    }
                });
                UNUserNotificationCenter::currentNotificationCenter()
                    .getNotificationSettingsWithCompletionHandler(&callback);
            })
            .is_err()
        {
            return false;
        }
        matches!(
            tokio::time::timeout(Duration::from_secs(3), receive).await,
            Ok(Ok(true))
        )
    }

    pub fn show(app: &AppHandle, version: &Version) {
        let identifier = format!("junadesk-update-{version}");
        let title = format!("JunaDesk {version} ist verfügbar");
        let _ = app.run_on_main_thread(move || {
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&title));
            content.setBody(&NSString::from_str(BODY));
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&identifier),
                &content,
                None,
            );
            let center = UNUserNotificationCenter::currentNotificationCenter();
            DELEGATE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let delegate = slot.get_or_insert_with(|| {
                    // The delegate is an NSObject subclass with no instance variables.
                    unsafe { msg_send![NotificationDelegate::class(), new] }
                });
                center.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
            });
            center.addNotificationRequest_withCompletionHandler(&request, None);
        });
    }
}

#[cfg(target_os = "macos")]
pub use macos::{allowed, request, show};

#[cfg(not(target_os = "macos"))]
use tauri_plugin_notification::{NotificationExt, PermissionState};

#[cfg(not(target_os = "macos"))]
pub async fn request(app: &AppHandle) -> Result<()> {
    match app.notification().request_permission() {
        Ok(PermissionState::Granted) => Ok(()),
        Ok(_) => Err("Erlaube JunaDesk Mitteilungen in den Systemeinstellungen.".into()),
        Err(_) => Err(PERMISSION_ERROR.into()),
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn allowed(app: &AppHandle) -> bool {
    matches!(
        app.notification().permission_state(),
        Ok(PermissionState::Granted)
    )
}

#[cfg(not(target_os = "macos"))]
pub fn show(app: &AppHandle, version: &Version) {
    let _ = app
        .notification()
        .builder()
        .title(format!("JunaDesk {version} ist verfügbar"))
        .body(BODY)
        .show();
}
