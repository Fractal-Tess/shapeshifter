use domain::LimitsSnapshotSet;
use egui::{ProgressBar, Ui};

pub fn limits_card(ui: &mut Ui, limits: Option<&LimitsSnapshotSet>) {
    ui.group(|ui| match limits {
        Some(limits) => {
            ui.label(format!("Plan: {:?}", limits.plan_type));
            if let Some(email) = limits.email.as_deref() {
                ui.label(format!("Email: {email}"));
            }
            render_window(ui, limits.primary_limit.primary.as_ref());
            render_window(ui, limits.primary_limit.secondary.as_ref());
        }
        None => {
            ui.label("No limits fetched yet.");
            ui.label("Expected source: GET /backend-api/codex/usage with Codex headers.");
        }
    });
}

fn render_window(ui: &mut Ui, window: Option<&domain::LimitWindow>) {
    if let Some(window) = window {
        ui.label(&window.label);
        ui.add(ProgressBar::new((window.used_percent / 100.0) as f32).show_percentage());
        if let Some(resets_at) = window.resets_at {
            ui.label(format!("Resets: {resets_at}"));
        }
    }
}
