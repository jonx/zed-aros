//! Text system for the AROS backend.
//!
//! `gpui_wgpu::CosmicTextSystem` is OS-independent (cosmic-text / swash /
//! fontdb only), so we reuse it directly with the same bundled fonts gpui_web
//! embeds. System-font enumeration is off; only the bundled faces are loaded.

use std::borrow::Cow;
use std::sync::Arc;

use gpui::PlatformTextSystem;
pub(crate) use gpui_wgpu::CosmicTextSystem;

static BUNDLED_FONTS: &[&[u8]] = &[
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"),
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Italic.ttf"),
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBoldItalic.ttf"),
    include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf"),
    include_bytes!("../../../assets/fonts/lilex/Lilex-Bold.ttf"),
    include_bytes!("../../../assets/fonts/lilex/Lilex-Italic.ttf"),
    include_bytes!("../../../assets/fonts/lilex/Lilex-BoldItalic.ttf"),
];

/// Build the text system, loading the bundled fonts.
pub(crate) fn new_text_system() -> Arc<dyn PlatformTextSystem> {
    let text_system = CosmicTextSystem::new_without_system_fonts("IBM Plex Sans");
    let fonts = BUNDLED_FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(error) = text_system.add_fonts(fonts) {
        log::error!("gpui_aros: failed to load bundled fonts: {error:#}");
    }
    Arc::new(text_system)
}
