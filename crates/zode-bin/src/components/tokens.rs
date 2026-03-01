pub mod colors {
    use eframe::egui::Color32;

    // Surfaces
    pub const SURFACE: Color32 = Color32::from_rgb(1, 1, 1);
    pub const SURFACE_DARK: Color32 = Color32::from_rgb(20, 20, 22);
    pub const SURFACE_RAISED: Color32 = Color32::from_rgb(28, 28, 30);
    pub const SURFACE_INTERACTIVE: Color32 = Color32::from_rgb(38, 38, 42);
    pub const PANEL_BG: Color32 = Color32::BLACK;

    // Borders
    pub const BORDER: Color32 = Color32::from_rgb(48, 48, 52);
    pub const BORDER_SUBTLE: Color32 = Color32::from_rgb(55, 55, 60);
    pub const BORDER_DIM: Color32 = Color32::from_rgb(50, 50, 55);

    // Text
    pub const TEXT_HEADING: Color32 = Color32::from_rgb(140, 140, 145);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(100, 100, 108);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(160, 160, 165);

    // Status
    pub const ERROR: Color32 = Color32::from_rgb(255, 80, 80);
    pub const WARN: Color32 = Color32::from_rgb(255, 200, 100);
    pub const CONNECTED: Color32 = Color32::from_rgb(46, 230, 176);
    pub const DISCONNECTED: Color32 = Color32::from_rgb(255, 80, 80);

    // Accent
    pub const ACCENT: Color32 = Color32::from_rgb(0, 180, 255);

    // Log levels
    pub const LOG_REJECT: Color32 = Color32::from_rgb(255, 100, 100);
    pub const LOG_GOSSIP: Color32 = Color32::from_rgb(100, 200, 255);
    pub const LOG_DISCOVERY: Color32 = Color32::from_rgb(100, 150, 255);
    pub const LOG_PEER_DISCONNECT: Color32 = Color32::from_rgb(255, 255, 100);
    pub const LOG_RELAY: Color32 = Color32::from_rgb(180, 130, 255);
    pub const LOG_DIAL_ERROR: Color32 = Color32::from_rgb(255, 140, 60);
    pub const LOG_RPC: Color32 = Color32::from_rgb(100, 220, 220);
    pub const LOG_SHUTDOWN: Color32 = Color32::from_rgb(200, 100, 255);
    pub const LOG_NORMAL: Color32 = Color32::from_rgb(200, 200, 200);

    // Visualization
    pub const VIZ_EDGE: Color32 = Color32::from_rgb(60, 60, 70);
    pub const VIZ_PEER_NODE: Color32 = Color32::from_rgb(120, 120, 130);
    pub const VIZ_TOOLTIP: Color32 = Color32::from_rgb(200, 200, 200);
    pub const VIZ_PANEL_TEXT: Color32 = Color32::from_rgb(190, 190, 190);
    pub const VIZ_OVERLAY_BG: Color32 = Color32::from_rgba_premultiplied(8, 8, 9, 200);
    pub const VIZ_GRID_LINE: Color32 = Color32::from_rgb(48, 48, 54);
    pub const VIZ_GRID_DOT: Color32 = Color32::from_rgb(72, 72, 80);
}

pub mod spacing {
    pub const XS: f32 = 2.0;
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const XL: f32 = 16.0;
    pub const XXL: f32 = 24.0;
    pub const XXXL: f32 = 32.0;
}

pub mod font_size {
    pub const SMALL: f32 = 9.0;
    pub const BODY: f32 = 10.0;
    pub const BUTTON: f32 = 10.0;
    pub const ACTION: f32 = 11.0;
    pub const SUBTITLE: f32 = 12.0;
    pub const HEADING: f32 = 10.0;
}

pub const WIDGET_HEIGHT: f32 = 24.0;
pub const ICON_SIZE: f32 = 16.0;
pub const STROKE_DEFAULT: f32 = 1.0;

pub fn default_stroke() -> eframe::egui::Stroke {
    eframe::egui::Stroke::new(STROKE_DEFAULT, colors::BORDER_SUBTLE)
}

pub fn border_stroke() -> eframe::egui::Stroke {
    eframe::egui::Stroke::new(STROKE_DEFAULT, colors::BORDER)
}
