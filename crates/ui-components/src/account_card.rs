use domain::{AccountProfile, AuthFile};
use egui::Ui;

pub fn account_card(ui: &mut Ui, profile: &AccountProfile, current_auth: Option<&AuthFile>) {
    ui.group(|ui| {
        ui.label(format!("Profile: {}", profile.label));
        ui.label(format!("Profile id: {}", profile.id));
        ui.label(format!("Source: {}", profile.source_path.display()));
        match current_auth {
            Some(auth_file) => {
                ui.label(format!(
                    "Current mode: {}",
                    auth_file.auth_mode.as_deref().unwrap_or("unknown")
                ));
                ui.label(format!(
                    "Current account id: {}",
                    auth_file.tokens.account_id.as_deref().unwrap_or("missing")
                ));
            }
            None => {
                ui.label("No current auth file loaded.");
            }
        }
    });
}
