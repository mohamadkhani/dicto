//! System tray via the StatusNotifierItem (SNI) protocol.
//!
//! Uses our forked `ksni` (github.com/mohamadkhani/ksni) which adds the
//! `ProvideXdgActivationToken` SNI method. Unlike the old `tray-icon` +
//! `libayatana-appindicator` + nested-GTK-pump stack, ksni speaks the SNI
//! D-Bus protocol directly (no GTK dependency), which is what actually works
//! on GNOME/Wayland and KDE.
//!
//! Tray actions are sent over an `mpsc` channel; the GPUI main loop polls it
//! (see `main.rs`). The "Quick Translate" item just sets the same
//! `TRAY_TRANSLATE_TRIGGERED` flag the `dicto --translate` IPC path uses, so
//! both share one trigger path handled in `app.rs`.

use std::sync::{Arc, Mutex, mpsc};

use ksni::{
    Category, Icon, ToolTip, Tray, TrayMethods,
    menu::{MenuItem, StandardItem},
};

/// The last compositor-provided activation token, shared between the SNI
/// service thread (which receives it) and the menu-item click handlers.
///
/// Forwarded to the popup window's `Window::activate_with_token` (provided by
/// the mohamadkhani/zed gpui fork) for authoritative focus on GNOME/Wayland.
pub type SharedToken = Arc<Mutex<Option<String>>>;

/// Actions the tray requests the GPUI main loop to perform.
#[derive(Debug)]
pub enum TrayAction {
    Show,
    QuickTranslate,
    Quit,
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

/// Spawn the ksni tray on a dedicated thread with its own current-thread
/// tokio runtime. Returns the channel the GPUI main loop polls for actions
/// and the shared activation-token slot.
pub fn spawn_tray() -> (mpsc::Receiver<TrayAction>, SharedToken) {
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

/// The tray icon: the inner dictionary card (front card + bold "A" +
/// definition lines) pre-rendered from `assets/tray-icon.svg` by librsvg at
/// 128×128 — see `assets/gen-tray-icon.sh`. Embedding a real SVG render
/// instead of rasterizing geometry by hand gives pixel-exact gradients and
/// smooth 8-bit anti-aliased edges at any tray scale. The raw RGBA bytes are
/// converted to the ARGB32 (network byte order) layout SNI `IconPixmap`
/// expects — the same `rotate_right(1)` the ksni docs show.
static TRAY_ICON: std::sync::LazyLock<Icon> = std::sync::LazyLock::new(|| {
    let rgba = include_bytes!("../../assets/tray-icon-128.raw");
    let mut data = rgba.to_vec();
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1); // RGBA → ARGB32
    }
    Icon {
        width: 128,
        height: 128,
        data,
    }
});

fn dicto_icon() -> Icon {
    TRAY_ICON.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_covers_the_canvas() {
        let icon = dicto_icon();
        assert_eq!(icon.width, 128);
        assert_eq!(icon.height, 128);
        assert_eq!(icon.data.len(), 128 * 128 * 4);

        // Byte order: ARGB32. The canvas corners are transparent padding —
        // all four bytes must be zero there.
        for (x, y) in [(0, 0), (127, 0), (0, 127), (127, 127)] {
            let i = (y * 128 + x) * 4;
            assert_eq!(&icon.data[i..i + 4], &[0, 0, 0, 0], "corner ({x},{y}) must be transparent");
        }

        // The card is scaled to fill the canvas (height-limited), so the
        // opaque area covers most of it.
        let opaque = icon
            .data
            .chunks_exact(4)
            .filter(|p| p[0] > 0)
            .count();
        let frac = opaque as f32 / (128 * 128) as f32;
        assert!(
            frac > 0.5,
            "tray icon only covers {:.1}% of the canvas — it looks small",
            frac * 100.0
        );

        // The "A" mark renders as near-black (#1a1b26) on the card.
        let dark = icon
            .data
            .chunks_exact(4)
            .filter(|p| p[0] == 255 && p[1] < 0x40 && p[2] < 0x40 && p[3] < 0x40)
            .count();
        assert!(dark > 10, "no dark 'A' mark rendered");
    }
}
