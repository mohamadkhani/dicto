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

use super::{SharedToken, TrayAction};

pub(super) fn spawn() -> (mpsc::Receiver<TrayAction>, SharedToken) {
    let (action_tx, action_rx) = mpsc::channel::<TrayAction>();
    let token: SharedToken = Arc::new(Mutex::new(None));

    let tray = DictoTray::new(action_tx, token.clone());

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
                    Ok(_handle) => {
                        tracing::info!("tray: ksni service spawned");
                        // Hold the runtime alive for the lifetime of the
                        // process. The handle owns the background D-Bus task;
                        // dropping it would tear down the tray.
                        std::future::pending::<()>().await;
                    }
                    Err(e) => {
                        tracing::warn!("tray: ksni spawn failed: {e}");
                    }
                }
            });
        })
        .expect("spawn ksni thread");

    (action_rx, token)
}

struct DictoTray {
    action_tx: mpsc::Sender<TrayAction>,
    #[allow(dead_code)]
    token: SharedToken,
}

impl DictoTray {
    fn new(action_tx: mpsc::Sender<TrayAction>, token: SharedToken) -> Self {
        Self { action_tx, token }
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
        let tx_translate = self.action_tx.clone();
        let tx_quit = self.action_tx.clone();

        vec![
            MenuItem::Standard(StandardItem {
                label: "Show Dictionary".into(),
                enabled: true,
                activate: Box::new(move |_this| {
                    let _ = tx_show.send(TrayAction::Show);
                }),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Quick Translate".into(),
                enabled: true,
                activate: Box::new(move |_this| {
                    let _ = tx_translate.send(TrayAction::QuickTranslate);
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Quit".into(),
                enabled: true,
                activate: Box::new(move |_this| {
                    let _ = tx_quit.send(TrayAction::Quit);
                }),
                ..Default::default()
            }),
        ]
    }
}

static TRAY_ICON: std::sync::LazyLock<Icon> = std::sync::LazyLock::new(|| Icon {
    width: super::icon::SIZE as i32,
    height: super::icon::SIZE as i32,
    data: super::icon::argb(),
});

fn dicto_icon() -> Icon {
    TRAY_ICON.clone()
}
