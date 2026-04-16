use crate::state::{AppState, BusyOperation, NoticeKind};
use chrono::Utc;
use domain::AccountProfile;
use egui::{Align, Button, Color32, ComboBox, Frame, Layout, Margin, RichText, ScrollArea, Ui};

const ACTION_BUTTON_SIZE: [f32; 2] = [150.0, 36.0];

pub fn top_bar(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal_wrapped(|ui| {
        ui.heading("Shapeshifter");
        ui.separator();
        ui.label(format!(
            "Selected host active: {}",
            state.selected_host_active_profile_label()
        ));
        ui.separator();

        if ui.button("Browser Login").clicked() {
            state.login_browser();
        }
        if ui.button("Import Account").clicked() {
            state.open_import_modal();
        }
        if ui.button("Start Device Login").clicked() {
            state.start_device_login();
        }
        if ui
            .add_enabled(
                state.device_prompt.is_some(),
                Button::new("Finish Device Login"),
            )
            .clicked()
        {
            state.finish_device_login();
        }

        ui.separator();

        if ui.button("Refresh Limits").clicked() {
            state.refresh_all_limits();
        }
        if state.is_busy(BusyOperation::RefreshLimits) {
            ui.spinner();
            ui.small("Refreshing…");
        }
        if ui.button("Reload Accounts").clicked() {
            state.refresh_disk_state();
        }

        let host_choices = state.host_choices();
        let mut selected_host_index = state.selected_host_index;
        ComboBox::from_label("Target Host")
            .selected_text(state.selected_remote_host_label())
            .show_ui(ui, |ui| {
                for (host_index, host_label) in &host_choices {
                    ui.selectable_value(&mut selected_host_index, *host_index, host_label);
                }
            });
        if selected_host_index != state.selected_host_index {
            state.set_selected_host_index(selected_host_index);
        }
        if ui
            .add_enabled(state.selected_host_is_remote(), Button::new("Sync Host"))
            .clicked()
        {
            state.sync_selected_remote();
        }
        if state.is_busy(BusyOperation::SyncHost) {
            ui.spinner();
            ui.small("Syncing host…");
        }
        if state.is_busy(BusyOperation::InspectHost) {
            ui.spinner();
            ui.small("Loading host…");
        }
    });

    if let Some(prompt) = state.device_prompt.as_ref() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Device login pending").strong());
            ui.label(format!("Visit {}", prompt.verification_url));
            ui.monospace(format!("Code: {}", prompt.user_code));
        });
    }

    if let Some(notice) = state.notice.clone() {
        let (fill, stroke, text_color) = match notice.kind {
            NoticeKind::Success => (
                Color32::from_rgb(24, 46, 28),
                Color32::from_rgb(90, 170, 110),
                Color32::from_rgb(190, 240, 200),
            ),
            NoticeKind::Error => (
                Color32::from_rgb(56, 24, 24),
                Color32::from_rgb(190, 80, 80),
                Color32::from_rgb(255, 210, 210),
            ),
        };

        Frame::group(ui.style())
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, stroke))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(notice.message).color(text_color).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Dismiss").clicked() {
                            state.clear_notice();
                        }
                    });
                });
            });
    }
}

pub fn main_panel(ui: &mut Ui, state: &mut AppState) {
    ui.heading(format!(
        "Accounts on {}",
        state.selected_remote_host_label()
    ));
    ui.add_space(8.0);

    ScrollArea::vertical()
        .id_salt("host-accounts-scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            for profile in state.selected_host_profiles.clone() {
                render_account_card(ui, state, &profile);
                ui.add_space(10.0);
            }
        });

    render_delete_modal(ui.ctx(), state);
    render_export_modal(ui.ctx(), state);
    render_import_modal(ui.ctx(), state);
}

fn render_account_card(ui: &mut Ui, state: &mut AppState, profile: &AccountProfile) {
    let is_active = state.is_profile_active_on_selected_host(profile);
    let frame = Frame::group(ui.style())
        .inner_margin(Margin::same(14))
        .stroke(if is_active {
            egui::Stroke::new(2.0, Color32::LIGHT_GREEN)
        } else {
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
        });

    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.heading(&profile.label);
                    if is_active {
                        ui.label(
                            RichText::new("ACTIVE")
                                .color(Color32::LIGHT_GREEN)
                                .strong(),
                        );
                    }
                });
                ui.monospace(format!(
                    "Account ID: {}",
                    profile
                        .auth_file
                        .tokens
                        .account_id
                        .as_deref()
                        .unwrap_or("missing")
                ));
                ui.small(format!("Source: {}", profile.source_path.display()));
                ui.add_space(8.0);
                render_limits(ui, state.profile_limits(&profile.id));
            });

            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if ui
                    .add(
                        Button::new(
                            RichText::new("🗑")
                                .size(20.0)
                                .color(Color32::from_rgb(240, 120, 120)),
                        )
                        .min_size([36.0, 36.0].into())
                        .fill(Color32::from_rgb(50, 20, 20)),
                    )
                    .clicked()
                {
                    state.prompt_delete_profile(profile.id.clone());
                }
                if ui
                    .add(Button::new("Export").min_size([110.0, 36.0].into()))
                    .clicked()
                {
                    state.open_export_profile(&profile.id);
                }
                if ui
                    .add(Button::new("Activate").min_size(ACTION_BUTTON_SIZE.into()))
                    .clicked()
                {
                    state.activate_profile_for_selected_host(&profile.id);
                }
            });
        });
    });
}

fn render_limits(ui: &mut Ui, limits: Option<&domain::LimitsSnapshotSet>) {
    egui::Grid::new(ui.next_auto_id())
        .num_columns(3)
        .spacing([12.0, 8.0])
        .striped(false)
        .show(ui, |ui| match limits {
            Some(limits) => {
                render_limit_row(ui, "5h", limits.primary_limit.primary.as_ref());
                ui.end_row();
                render_limit_row(ui, "Weekly", limits.primary_limit.secondary.as_ref());
                ui.end_row();
            }
            None => {
                render_limit_row(ui, "5h", None);
                ui.end_row();
                render_limit_row(ui, "Weekly", None);
                ui.end_row();
            }
        });
}

fn render_limit_row(ui: &mut Ui, label: &str, window: Option<&domain::LimitWindow>) {
    ui.label(RichText::new(format!("{label}:")).strong());
    match window {
        Some(window) => {
            let left_percent = (100.0 - window.used_percent).clamp(0.0, 100.0);
            ui.add(
                egui::ProgressBar::new((left_percent / 100.0) as f32)
                    .text(format!("{left_percent:.0}% left"))
                    .desired_width(260.0),
            );
            ui.label(
                RichText::new(format_reset(window.resets_at))
                    .size(16.0)
                    .strong(),
            );
        }
        None => {
            ui.label("not fetched yet");
            ui.small("-");
        }
    }
}

fn render_delete_modal(ctx: &egui::Context, state: &mut AppState) {
    let Some(profile_id) = state.pending_delete_profile.clone() else {
        return;
    };
    let profile_label = state
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .map(|profile| profile.label.clone())
        .unwrap_or(profile_id);

    egui::Window::new("Confirm Delete")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!("Delete account `{profile_label}`?"));
            ui.label("This removes the saved profile JSON from the managed accounts directory.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    state.cancel_delete_profile();
                }
                if ui
                    .add(
                        Button::new(RichText::new("Delete").color(Color32::WHITE))
                            .fill(Color32::from_rgb(170, 35, 35)),
                    )
                    .clicked()
                {
                    state.confirm_delete_profile();
                }
            });
        });
}

fn render_export_modal(ctx: &egui::Context, state: &mut AppState) {
    let Some(profile_label) = state.export_profile_label.clone() else {
        return;
    };

    egui::Window::new("Export Account")
        .collapsible(false)
        .resizable(true)
        .default_width(720.0)
        .default_height(420.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!("Copy the account JSON for `{profile_label}`."));
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::multiline(&mut state.export_text)
                    .desired_width(f32::INFINITY)
                    .desired_rows(18),
            );
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                state.close_export_modal();
            }
        });
}

fn render_import_modal(ctx: &egui::Context, state: &mut AppState) {
    if !state.import_modal_open {
        return;
    }

    egui::Window::new("Import Account")
        .collapsible(false)
        .resizable(true)
        .default_width(720.0)
        .default_height(420.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Paste an account auth JSON payload.");
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::multiline(&mut state.import_text)
                    .desired_width(f32::INFINITY)
                    .desired_rows(18),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    state.close_import_modal();
                }
                if ui.button("Import").clicked() {
                    state.import_profile_from_text();
                }
            });
        });
}

fn format_reset(resets_at: Option<chrono::DateTime<Utc>>) -> String {
    let Some(resets_at) = resets_at else {
        return "Reset unknown".into();
    };
    let now = Utc::now();
    let duration = resets_at.signed_duration_since(now);
    let total_hours = duration.num_minutes().max(0) as f64 / 60.0;
    if total_hours >= 24.0 {
        format!(
            "Resets in {:.1} hours | {}",
            total_hours,
            resets_at.format("%d-%m")
        )
    } else {
        format!(
            "Resets in {:.1} hours | {}",
            total_hours,
            resets_at.format("%d-%m")
        )
    }
}
