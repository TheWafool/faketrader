#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("FakeTrader")
        .with_app_id("faketrader")
        .with_inner_size([1400.0, 860.0])
        .with_min_inner_size([960.0, 600.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("FakeTrader", options, Box::new(|cc| Ok(Box::new(faketrader::ui::App::new(cc)))))
}
