use crate::state::AppState;
use crate::views;

pub struct ShapeshifterApp {
    state: AppState,
}

impl ShapeshifterApp {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            state: AppState::load()?,
        })
    }
}

impl eframe::App for ShapeshifterApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.state.poll_background();
        if self.state.busy_operation.is_some() || self.state.notice.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        egui::TopBottomPanel::top("top-bar").show(ctx, |ui| views::top_bar(ui, &mut self.state));
        egui::CentralPanel::default().show(ctx, |ui| views::main_panel(ui, &mut self.state));
    }
}
