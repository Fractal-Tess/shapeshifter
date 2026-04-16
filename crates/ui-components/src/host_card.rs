use domain::{HostTarget, ManagedHost};
use egui::Ui;

pub fn host_card(ui: &mut Ui, host: &ManagedHost) {
    ui.group(|ui| {
        ui.label(format!("Target: {}", host.label));
        match &host.target {
            HostTarget::Local { auth_file_path } => {
                ui.label(format!("Local file: {}", auth_file_path.display()));
            }
            HostTarget::Remote(remote) => {
                ui.label(format!("SSH host: {}", remote.ssh_alias));
                ui.label(format!("Remote file: {}", remote.auth_file_path.display()));
            }
        }
    });
}
