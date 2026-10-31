//! Linux system tray via the StatusNotifierItem (SNI) protocol.
//!
//! Uses our forked `ksni` (github.com/mohamadkhani/ksni) which adds the
//! `ProvideXdgActivationToken` SNI method. Unlike the old `tray-icon` +
//! `libayatana-appindicator` + nested-GTK-pump stack, ksni speaks the SNI
//! D-Bus protocol directly (no GTK dependency), which is what actually works
//! on GNOME/Wayland and KDE.

use std::sync::{Arc, Mutex, mpsc};

use ksni::{
    Category, Icon, ToolTip, Tray, TrayMethods,
    menu::{MenuItem, StandardItem},
};

use super::{MenuRefresh, SharedToken, SharedVisibility, TrayAction};

pub(super) fn spawn(
    visibility: SharedVisibility,
) -> (mpsc::Receiver<TrayAction>, SharedToken, MenuRefresh) {
    let (action_tx, action_rx) = mpsc::channel::<TrayAction>();
    let token: SharedToken = Arc::new(Mutex::new(None));
    // ksni rebuilds its cached menu only when the service is poked, so a
    // visibility flip has to wake the tray thread (see the loop below).
    let (refresh_tx, mut refresh_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let refresh: MenuRefresh = Arc::new(move || {
        let _ = refresh_tx.send(());
    });

    let tray = DictoTray::new(action_tx, token.clone(), visibility);

    std::thread::Builder::new()
        .name("ksni-tray".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!("tray: failed to build tokio runtime: {e}");
                    return;
                }
            };

            runtime.block_on(async move {
                match tray.spawn().await {
                    Ok(handle) => {
                        tracing::info!("tray: ksni service spawned");
                        // The handle owns the background D-Bus task —
                        // dropping it would tear down the tray — so it lives
                        // in this loop for the process lifetime. Each
                        // refresh makes the service re-read `menu()` and
                        // push the new layout to the host.
                        while refresh_rx.recv().await.is_some() {
                            let _ = handle.update(|_| {}).await;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("tray: ksni spawn failed: {e}");
                    }
                }
            });
        })
        .expect("spawn ksni thread");

    (action_rx, token, refresh)
}

struct DictoTray {
    action_tx: mpsc::Sender<TrayAction>,
    token: SharedToken,
    visibility: SharedVisibility,
}

impl DictoTray {
    fn new(
        action_tx: mpsc::Sender<TrayAction>,
        token: SharedToken,
        visibility: SharedVisibility,
    ) -> Self {
        Self {
            action_tx,
            token,
            visibility,
        }
    }
}

impl Tray for DictoTray {
    fn id(&self) -> String {
        "dicto".into()
    }
    fn title(&self) -> String {
        "Dicto".into()
    }
    fn category(&self) -> Category {
        Category::ApplicationStatus
    }
    fn icon_name(&self) -> String {
        // Deliberately empty: GNOME's AppIndicator extension prefers
        // `IconName` over `IconPixmap` whenever the name resolves in the icon
        // theme, which would shadow our pixel icon. Returning "" forces it to
        // render our `icon_pixmap`.
        String::new()
    }
    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![dicto_icon()]
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Dicto".into(),
            description: "Dictionary & quick translate".into(),
            ..Default::default()
        }
    }

    fn on_activation_token(&mut self, token: String) {
        tracing::debug!(
            chars = token.len(),
            "tray: ProvideXdgActivationToken received"
        );
        if let Ok(mut g) = self.token.lock() {
            *g = Some(token);
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let tx_show = self.action_tx.clone();

        let mut items: Vec<MenuItem<Self>> = vec![MenuItem::Standard(StandardItem {
            label: "Show Dictionary".into(),
            enabled: true,
            activate: Box::new(move |this| {
                // The host delivered the token (if it supports
                // ProvideXdgActivationToken) just before this click;
                // capture it here so it travels with THIS action (the
                // shared slot is drained by translate/lookup at poll
                // time and would race a rapid second click).
                let token = this.token.lock().ok().and_then(|mut g| g.take());
                let _ = tx_show.send(TrayAction::Show { token });
            }),
            ..Default::default()
        })];

        // The quick-translate / lookup items exist only while their feature
        // is enabled (see `sync_menu_visibility`). Their configured hotkey
        // is appended to the label as a string — GNOME's AppIndicator host
        // ignores the SNI `shortcut` property entirely, so the label is the
        // only place every host renders it (display hint only: the global
        // hotkey backend is what actually fires the action, and we must not
        // set the shortcut field as well or KDE would show it twice).
        if self.visibility.quick_translate() {
            let tx_translate = self.action_tx.clone();
            let hotkey = self.visibility.translate_hotkey();
            items.push(MenuItem::Standard(StandardItem {
                label: labeled("Quick Translate", &hotkey),
                enabled: true,
                activate: Box::new(move |_this| {
                    let _ = tx_translate.send(TrayAction::QuickTranslate);
                }),
                ..Default::default()
            }));
        }
        if self.visibility.quick_lookup() {
            let tx_lookup = self.action_tx.clone();
            let hotkey = self.visibility.lookup_hotkey();
            items.push(MenuItem::Standard(StandardItem {
                label: labeled("Look Up Word", &hotkey),
                enabled: true,
                activate: Box::new(move |_this| {
                    let _ = tx_lookup.send(TrayAction::QuickLookup);
                }),
                ..Default::default()
            }));
        }

        items.push(MenuItem::Separator);

        let tx_quit = self.action_tx.clone();
        items.push(MenuItem::Standard(StandardItem {
            label: "Quit".into(),
            enabled: true,
            activate: Box::new(move |_this| {
                let _ = tx_quit.send(TrayAction::Quit);
            }),
            ..Default::default()
        }));

        items
    }
}

static TRAY_ICON: std::sync::LazyLock<Icon> = std::sync::LazyLock::new(|| Icon {
    width: super::icon::SIZE as i32,
    height: super::icon::SIZE as i32,
    data: super::icon::argb(),
});

/// The item label with the hotkey string appended, e.g.
/// `"Quick Translate (Ctrl+Alt+D)"` — the same string the settings panel
/// shows. A blank hotkey leaves the label bare.
fn labeled(label: &str, hotkey: &str) -> String {
    let hotkey = hotkey.trim();
    if hotkey.is_empty() {
        label.to_owned()
    } else {
        format!("{label} ({hotkey})")
    }
}

fn dicto_icon() -> Icon {
    TRAY_ICON.clone()
}
