#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod chat;
mod components;
mod helpers;
mod identity;
mod render;
mod render_storage;
mod settings;
mod state;
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
    let pad = (sw / 4) as i64;
    let out_w = sw as i64 + pad * 2;
    let out_h = sh as i64 + pad * 2;
    let mut rgba = vec![0u8; (out_w * out_h * 4) as usize];
    for y in 0..sh {
        for x in 0..sw {
            let px = src.get_pixel(x, y);
            let dst_x = x as i64 + pad;
            let dst_y = y as i64 + pad;
            let i = ((dst_y * out_w + dst_x) * 4) as usize;
            rgba[i] = px[0];
            rgba[i + 1] = px[1];
            rgba[i + 2] = px[2];
            rgba[i + 3] = px[3];
        }
    }
    egui::IconData {
        width: out_w as u32,
        height: out_h as u32,
        rgba,
    }
}

fn configure_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::BLACK;
    visuals.window_fill = egui::Color32::from_rgb(28, 28, 30);
    visuals.extreme_bg_color = egui::Color32::from_rgb(20, 20, 22);
    visuals.faint_bg_color = egui::Color32::from_rgb(28, 28, 30);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(28, 28, 30);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(38, 38, 42);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(50, 50, 55);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.expansion = 0.0;
    visuals.widgets.active.bg_fill = egui::Color32::BLACK;
    visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.active.expansion = 0.0;
    visuals.selection.bg_fill = egui::Color32::WHITE;
    visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
    ctx.set_visuals(visuals);

    ctx.style_mut(|s| {
        s.spacing.interact_size.y = 20.0;
        use egui::{FontId, TextStyle};
        s.text_styles
            .insert(TextStyle::Body, FontId::proportional(11.0));
        s.text_styles
            .insert(TextStyle::Button, FontId::proportional(11.0));
        s.text_styles
            .insert(TextStyle::Small, FontId::proportional(9.0));
        s.text_styles
            .insert(TextStyle::Monospace, FontId::monospace(10.0));
    });
}
