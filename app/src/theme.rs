use eframe::egui::{Color32, FontFamily, FontId};
use engine::NodeKind;

pub const BG: Color32 = Color32::from_rgb(10, 10, 12);
pub const PANEL: Color32 = Color32::from_rgb(16, 16, 19);
pub const DOT: Color32 = Color32::from_rgb(28, 28, 33);
pub const LINE: Color32 = Color32::from_rgb(52, 52, 60);
pub const DIM: Color32 = Color32::from_rgb(95, 95, 105);
pub const TEXT: Color32 = Color32::from_rgb(200, 200, 205);
pub const BRIGHT: Color32 = Color32::from_rgb(240, 240, 240);
/// Acid yellow: selection and focus.
pub const ACCENT: Color32 = Color32::from_rgb(214, 255, 64);
pub const ERROR: Color32 = Color32::from_rgb(255, 80, 80);

pub fn kind_color(kind: NodeKind) -> Color32 {
    match kind {
        NodeKind::Seq => Color32::from_rgb(70, 220, 230),
        NodeKind::Synth => Color32::from_rgb(255, 70, 160),
        NodeKind::Amp => Color32::from_rgb(170, 170, 180),
        NodeKind::Delay => Color32::from_rgb(150, 110, 255),
        NodeKind::Out => Color32::from_rgb(90, 255, 140),
    }
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}
