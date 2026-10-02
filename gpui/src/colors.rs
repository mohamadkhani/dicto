use gpui::Hsla;

pub fn bg() -> Hsla {
    gpui::rgb(0x0f1117).into()
}

pub fn surface() -> Hsla {
    gpui::rgb(0x1a1d27).into()
}

/// Slightly lighter than `surface` — used for nested controls (buttons, inputs).
pub fn surface_alt() -> Hsla {
    gpui::rgb(0x222634).into()
}

/// Hover background for interactive elements.
pub fn hover() -> Hsla {
    gpui::rgb(0x2a2f3e).into()
}

pub fn primary() -> Hsla {
    gpui::rgb(0x7aa2f7).into()
}

pub fn text() -> Hsla {
    gpui::rgb(0xc0caf5).into()
}

pub fn text_secondary() -> Hsla {
    gpui::rgb(0x565f89).into()
}

pub fn border() -> Hsla {
    gpui::rgb(0x292e42).into()
}

pub fn success() -> Hsla {
    gpui::rgb(0x50c878).into()
}

pub fn error() -> Hsla {
    gpui::rgb(0xe05050).into()
}

pub fn update() -> Hsla {
    gpui::rgb(0xe09050).into()
}
