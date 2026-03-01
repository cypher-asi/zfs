#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod components;
mod helpers;
mod identity;
mod interlink;
mod profile;
mod render;
mod render_storage;
mod settings;
mod setup;
mod state;
mod vault;
mod visualization;
mod visualization_render;

use std::sync::Arc;

use eframe::egui;
use tokio::runtime::Runtime;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let rt = Runtime::new()?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ZODE")
            .with_inner_size([820.0, 832.0])
            .with_icon(Arc::new(load_icon()))
            .with_decorations(false)
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "ZODE",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            configure_fonts(&cc.egui_ctx);
            configure_theme(&cc.egui_ctx);
            Ok(Box::new(app::ZodeApp::new(rt)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    Ok(())
}

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "inter".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/Inter-Regular.ttf"
        ))),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

fn load_icon() -> egui::IconData {
    let src = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("bad icon png")
        .to_rgba8();
    let (sw, sh) = (src.width(), src.height());

    let pad = (sw.min(sh) as f64 * 0.10) as u32;
    let out_w = sw + pad * 2;
    let out_h = sh + pad * 2;
    let r = (out_w.min(out_h) as f64) * 0.18;

    let mut rgba = vec![0u8; (out_w * out_h * 4) as usize];

    for y in 0..out_h {
        for x in 0..out_w {
            let i = ((y * out_w + x) * 4) as usize;
            let a = corner_alpha(
                x as f64 + 0.5,
                y as f64 + 0.5,
                out_w as f64,
                out_h as f64,
                r,
            );
            if a <= 0.0 {
                continue;
            }

            let (r_val, g_val, b_val) = if x >= pad && x < pad + sw && y >= pad && y < pad + sh {
                let px = src.get_pixel(x - pad, y - pad);
                (px[0], px[1], px[2])
            } else {
                (0, 0, 0)
            };

            rgba[i] = r_val;
            rgba[i + 1] = g_val;
            rgba[i + 2] = b_val;
            rgba[i + 3] = (a * 255.0).round() as u8;
        }
    }

    egui::IconData {
        width: out_w,
        height: out_h,
        rgba,
    }
}

fn corner_alpha(cx: f64, cy: f64, w: f64, h: f64, r: f64) -> f64 {
    let (kx, ky) = if cx < r && cy < r {
        (r, r)
    } else if cx > w - r && cy < r {
        (w - r, r)
    } else if cx < r && cy > h - r {
        (r, h - r)
    } else if cx > w - r && cy > h - r {
        (w - r, h - r)
    } else {
        return 1.0;
    };
    let dist = ((cx - kx).powi(2) + (cy - ky).powi(2)).sqrt();
    (r + 0.5 - dist).clamp(0.0, 1.0)
}

fn configure_theme(ctx: &egui::Context) {
    use components::tokens::{colors, font_size};

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = colors::PANEL_BG;
    visuals.window_fill = colors::SURFACE_RAISED;
    visuals.extreme_bg_color = colors::SURFACE_DARK;
    visuals.faint_bg_color = colors::SURFACE_RAISED;
    visuals.widgets.noninteractive.bg_fill = colors::SURFACE_RAISED;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.inactive.bg_fill = colors::SURFACE_INTERACTIVE;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.bg_fill = colors::BORDER_DIM;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.expansion = 0.0;
    visuals.widgets.active.bg_fill = colors::PANEL_BG;
    visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.active.expansion = 0.0;
    visuals.selection.bg_fill = egui::Color32::WHITE;
    visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
    ctx.set_visuals(visuals);

    ctx.style_mut(|s| {
        s.spacing.interact_size.y = 20.0;
        use egui::{FontId, TextStyle};
        s.text_styles
            .insert(TextStyle::Body, FontId::proportional(font_size::ACTION));
        s.text_styles
            .insert(TextStyle::Button, FontId::proportional(font_size::ACTION));
        s.text_styles
            .insert(TextStyle::Small, FontId::proportional(font_size::SMALL));
        s.text_styles
            .insert(TextStyle::Monospace, FontId::monospace(font_size::BODY));
    });
}
