//! robat-synth: a keyboard-driven node synth that streams multichannel
//! audio into Logic Pro.

mod audio;
mod draw;
mod theme;
mod ui;

use eframe::egui;

struct SynthApp(ui::App);

impl eframe::App for SynthApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let app = &mut self.0;
        for input in ui::collect_inputs(ui.ctx()) {
            app.handle(input);
        }
        if app.quit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        app.session.pump();
        draw::draw(app, ui);
        // Visuals animate continuously.
        ui.ctx().request_repaint();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self) {
        self.0.autosave();
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("robat synth")
            .with_inner_size([1280.0, 780.0])
            .with_min_inner_size([720.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "robat-synth",
        options,
        Box::new(|cc| {
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(SynthApp(ui::App::new())))
        }),
    )
}
