#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod store;
mod vpk;
mod wav;

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--verify") {
        verify();
        return Ok(());
    }
    let icon = egui::IconData {
        rgba: include_bytes!("../icon64.rgba").to_vec(),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 720.0])
            .with_min_inner_size([920.0, 540.0])
            .with_icon(std::sync::Arc::new(icon))
            .with_title("TF2 Sound Studio"),
        ..Default::default()
    };
    eframe::run_native(
        "TF2 Sound Studio",
        options,
        Box::new(|cc| {
            let mut style = (*cc.egui_ctx.style()).clone();
            style.visuals = egui::Visuals::dark();
            style.visuals.panel_fill = egui::Color32::from_rgb(24, 26, 31);
            style.visuals.window_fill = egui::Color32::from_rgb(28, 30, 36);
            style.visuals.extreme_bg_color = egui::Color32::from_rgb(18, 20, 24);
            style.visuals.selection.bg_fill = egui::Color32::from_rgb(70, 110, 200);
            style.visuals.widgets.noninteractive.bg_stroke.color =
                egui::Color32::from_rgb(45, 48, 56);
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(10.0, 5.0);
            style.interaction.selectable_labels = false;
            cc.egui_ctx.set_style(style);
            Ok(Box::new(app::App::new(cc)))
        }),
    )
}

fn verify() {
    let dir = match app::detect_tf_dir() {
        Some(d) => d,
        None => {
            println!("TF2 install not found");
            return;
        }
    };
    for pack in [vpk::PACK_MISC, vpk::PACK_VO] {
        match vpk::index_pack(&dir, pack) {
            Ok(list) => {
                println!("pack {} -> {} entries", pack, list.len());
                if let Some(e) = list.iter().find(|e| e.path.ends_with("wood1.wav")) {
                    let data = vpk::read_entry(&dir, e).unwrap_or_default();
                    let info = wav::info(&data);
                    println!(
                        "  {} len={} wav={:?} dur={:?}",
                        e.path,
                        data.len(),
                        info.map(|w| (w.fmt, w.channels, w.rate, w.bits)),
                        wav::duration(&data)
                    );
                }
            }
            Err(e) => println!("pack {} FAILED: {}", pack, e),
        }
    }
    println!("writable: {}", vpk::can_write(&dir, vpk::PACK_MISC));
}
