use crate::app::{App, FocusArea, Modal, NoticeKind};
use chrono::Utc;
use domain::LimitWindow;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},

    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap,
    },
};

const ACCENT: Color = Color::Cyan;
const ACTIVE_COLOR: Color = Color::Green;
const ERROR_COLOR: Color = Color::Red;
const DIM: Color = Color::DarkGray;
const SURFACE: Color = Color::Rgb(30, 30, 40);

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // header
            Constraint::Length(3), // status bar
            Constraint::Min(0),   // main content
            Constraint::Length(2), // help bar
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_status_bar(f, app, chunks[1]);
    draw_main(f, app, chunks[2]);
    draw_help_bar(f, app, chunks[3]);

    if let Some(notice) = &app.notice {
        draw_notice(f, notice);
    }

    match app.modal {
        Some(Modal::DeleteConfirm) => draw_delete_modal(f, app),
        Some(Modal::Import) => draw_import_modal(f, app),
        Some(Modal::Help) => draw_help_modal(f),
        Some(Modal::UpdateConfirm) => draw_update_modal(f, app),
        None => {}
    }

    // Update badge (bottom-right, non-intrusive)
    if app.available_update.is_some() && app.modal != Some(Modal::UpdateConfirm) {
        draw_update_badge(f, app);
    }

    if app.host_selector_open {
        draw_host_selector(f, app);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(DIM));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Animated gradient title
    let title = "▌ S H A P E S H I F T E R ▐";
    let gradient_colors = [
        Color::Cyan,
        Color::LightCyan,
        Color::Blue,
        Color::LightBlue,
        Color::Magenta,
        Color::LightMagenta,
        Color::Cyan,
    ];

    let tick = app.tick as usize;
    let title_spans: Vec<Span> = title
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let color_idx = (i + tick / 2) % gradient_colors.len();
            Span::styled(
                c.to_string(),
                Style::default().fg(gradient_colors[color_idx]).bold(),
            )
        })
        .collect();

    let title_line = Line::from(title_spans);
    let title_width = title.len() as u16;
    let x_offset = inner.width.saturating_sub(title_width) / 2;
    let title_area = Rect::new(inner.x + x_offset, inner.y, title_width.min(inner.width), 1);
    f.render_widget(Paragraph::new(title_line), title_area);

    // Animated underline
    let wave_chars = ['─', '═', '━', '═'];
    let underline_width = (title_width + 4).min(inner.width);
    let ux = inner.width.saturating_sub(underline_width) / 2;
    let underline: String = (0..underline_width as usize)
        .map(|i| wave_chars[(i + tick / 3) % wave_chars.len()])
        .collect();
    let underline_area = Rect::new(inner.x + ux, inner.y + 1, underline_width, 1);
    f.render_widget(
        Paragraph::new(Span::styled(underline, Style::default().fg(Color::Rgb(60, 60, 80)))),
        underline_area,
    );

    // Status line below title
    let mut status_spans: Vec<Span> = Vec::new();

    if let Some(op) = app.busy_operation {
        let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let frame = spinner[tick % spinner.len()];
        status_spans.push(Span::styled(
            format!("{frame} {} ", op.label()),
            Style::default().fg(Color::Yellow).bold(),
        ));
    }

    if let Some(prompt) = &app.device_prompt {
        if !status_spans.is_empty() {
            status_spans.push(Span::raw(" "));
        }
        status_spans.push(Span::styled(" DEVICE LOGIN ", Style::default().fg(Color::Black).bg(Color::Yellow).bold()));
        status_spans.push(Span::raw(format!("  {}  code: ", prompt.verification_url)));
        status_spans.push(Span::styled(&prompt.user_code, Style::default().fg(Color::White).bold()));
    }

    if !status_spans.is_empty() {
        let status_line = Line::from(status_spans);
        let status_area = Rect::new(inner.x, inner.y + 2, inner.width, 1);
        f.render_widget(Paragraph::new(status_line).alignment(ratatui::layout::Alignment::Center), status_area);
    }
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(DIM))
        .padding(Padding::horizontal(1));

    let host_label = app.selected_host_label();
    let active_label = app.active_profile_label();
    let profile_count = app.selected_host_profiles.len();

    let mut line = Line::from(vec![
        Span::styled("HOST ", Style::default().fg(DIM)),
        Span::styled(&host_label, Style::default().fg(ACCENT).bold()),
        Span::raw("  "),
        Span::styled("ACTIVE ", Style::default().fg(DIM)),
        Span::styled(&active_label, Style::default().fg(ACTIVE_COLOR).bold()),
        Span::raw("  "),
        Span::styled("PROFILES ", Style::default().fg(DIM)),
        Span::styled(
            profile_count.to_string(),
            Style::default().fg(Color::White).bold(),
        ),
    ]);
    let marked = app.marked_profiles.len();
    if marked > 0 {
        line.spans.push(Span::raw("  "));
        line.spans.push(Span::styled("MARKED ", Style::default().fg(DIM)));
        line.spans.push(Span::styled(
            marked.to_string(),
            Style::default().fg(Color::Yellow).bold(),
        ));
    }

    f.render_widget(Paragraph::new(line).block(block), area);
}

const CARD_HEIGHT: u16 = 5;

fn draw_main(f: &mut Frame, app: &App, area: Rect) {
    let has_search = app.search_active || !app.search_query.is_empty();
    let search_height = if has_search { 1 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(search_height),
            Constraint::Min(0),
        ])
        .split(area);

    // Search bar
    if has_search {
        let style = if app.search_active {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(DIM)
        };
        let match_count = app.visible_profiles().len();
        let line = Line::from(vec![
            Span::styled(" /", style.bold()),
            Span::styled(&app.search_query, Style::default().fg(Color::White)),
            if app.search_active {
                Span::styled("_", Style::default().fg(ACCENT).add_modifier(Modifier::SLOW_BLINK))
            } else {
                Span::raw("")
            },
            Span::styled(
                format!("  ({match_count} match{})", if match_count == 1 { "" } else { "es" }),
                Style::default().fg(DIM),
            ),
        ]);
        f.render_widget(Paragraph::new(line), chunks[0]);
    }

    let main_area = chunks[1];
    let focused = app.focus == FocusArea::ProfileList && app.modal.is_none();
    let border_color = if focused { ACCENT } else { DIM };

    let block = Block::default()
        .title(format!(" Accounts [{}] ", app.selected_host_label()))
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(main_area);
    f.render_widget(block, main_area);

    let visible = app.visible_profiles();
    if visible.is_empty() {
        let msg = if has_search {
            "  No matching profiles"
        } else {
            "  No profiles found"
        };
        f.render_widget(
            Paragraph::new(msg).style(Style::default().fg(DIM)),
            inner,
        );
        return;
    }

    // Find position of selected profile in visible list for scrolling
    let sel_visible_pos = visible
        .iter()
        .position(|(i, _)| *i == app.selected_profile_index)
        .unwrap_or(0) as u16;

    let visible_cards = inner.height / CARD_HEIGHT;
    let scroll_offset = if visible_cards == 0 {
        0
    } else if sel_visible_pos < visible_cards {
        0
    } else {
        sel_visible_pos - visible_cards + 1
    };

    let mut y = inner.y;
    for (vi, (real_idx, profile)) in visible.iter().enumerate() {
        let vi = vi as u16;
        if vi < scroll_offset {
            continue;
        }
        if y + CARD_HEIGHT > inner.y + inner.height {
            break;
        }
        let card_area = Rect::new(inner.x, y, inner.width, CARD_HEIGHT);
        draw_account_card(f, app, profile, *real_idx, card_area);
        y += CARD_HEIGHT;
    }

    // Action bar for selected profile at the bottom if focused
    if app.focus == FocusArea::ActionBar {
        let bar_area = Rect::new(inner.x + 1, inner.y + inner.height.saturating_sub(1), inner.width.saturating_sub(2), 1);
        draw_actions_bar(f, app, bar_area);
    }
}

fn draw_account_card(f: &mut Frame, app: &App, profile: &domain::AccountProfile, index: usize, area: Rect) {
    let is_active = app.is_profile_active(profile);
    let is_selected = index == app.selected_profile_index;
    let focused = app.focus == FocusArea::ProfileList && app.modal.is_none();
    let limits = app.profile_limits(&profile.id);

    // Card border
    let border_color = if is_selected && focused {
        ACCENT
    } else if is_active {
        ACTIVE_COLOR
    } else {
        Color::Rgb(50, 50, 60)
    };

    let card_block = Block::default()
        .borders(Borders::ALL)
        .border_type(if is_selected && focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(border_color))
        .padding(Padding::horizontal(1));

    let inner = card_block.inner(area);
    f.render_widget(card_block, area);

    // Row 1: mark + active + name + account id
    let is_marked = app.marked_profiles.contains(&profile.id);
    let mut name_spans = vec![];
    // Mark checkbox
    if is_marked {
        name_spans.push(Span::styled("◆ ", Style::default().fg(Color::Yellow).bold()));
    } else {
        name_spans.push(Span::styled("◇ ", Style::default().fg(Color::Rgb(50, 50, 60))));
    }
    // Active indicator
    if is_active {
        name_spans.push(Span::styled("● ", Style::default().fg(ACTIVE_COLOR)));
    } else {
        name_spans.push(Span::styled("  ", Style::default().fg(DIM)));
    }
    name_spans.push(Span::styled(
        &profile.label,
        Style::default().fg(Color::White).bold(),
    ));
    if is_active {
        name_spans.push(Span::styled(" ACTIVE", Style::default().fg(ACTIVE_COLOR).bold()));
    }
    // pad then account id
    let acct_id = profile
        .auth_file
        .tokens
        .account_id
        .as_deref()
        .unwrap_or("");
    if !acct_id.is_empty() {
        name_spans.push(Span::styled("  ", Style::default().fg(DIM)));
        name_spans.push(Span::styled(acct_id, Style::default().fg(DIM)));
    }

    // Row 2: 5h limit inline
    let row_5h = format_limit_inline("5h", limits.and_then(|l| l.primary_limit.primary.as_ref()));
    // Row 3: weekly limit inline
    let row_wk = format_limit_inline("Wk", limits.and_then(|l| l.primary_limit.secondary.as_ref()));

    let lines = vec![
        Line::from(name_spans),
        Line::from(row_5h),
        Line::from(row_wk),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn format_limit_inline<'a>(label: &'a str, window: Option<&LimitWindow>) -> Vec<Span<'a>> {
    let mut spans = vec![
        Span::styled(format!("  {label:>2} "), Style::default().fg(DIM).bold()),
    ];

    match window {
        Some(w) => {
            let remaining = (100.0 - w.used_percent).clamp(0.0, 100.0);
            let color = if remaining > 50.0 {
                ACTIVE_COLOR
            } else if remaining > 20.0 {
                Color::Yellow
            } else {
                ERROR_COLOR
            };

            // Text-based bar: 20 chars wide
            let filled = ((remaining / 100.0) * 20.0).round() as usize;
            let empty = 20 - filled;
            spans.push(Span::styled(
                "█".repeat(filled),
                Style::default().fg(color),
            ));
            spans.push(Span::styled(
                "░".repeat(empty),
                Style::default().fg(Color::Rgb(40, 40, 50)),
            ));
            spans.push(Span::styled(
                format!(" {remaining:>3.0}%", remaining = remaining),
                Style::default().fg(color).bold(),
            ));

            let reset = format_reset(w.resets_at);
            spans.push(Span::styled(
                format!("  {reset}"),
                Style::default().fg(DIM),
            ));
        }
        None => {
            spans.push(Span::styled("not fetched", Style::default().fg(DIM)));
        }
    }

    spans
}

fn draw_actions_bar(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusArea::ActionBar && app.modal.is_none();
    let actions = ["Activate", "Export", "Delete"];

    let spans: Vec<Span> = actions
        .iter()
        .enumerate()
        .flat_map(|(i, action)| {
            let is_sel = focused && i == app.action_bar_index;
            let style = if is_sel {
                Style::default().fg(Color::Black).bg(ACCENT).bold()
            } else {
                Style::default().fg(Color::White).bg(Color::Rgb(50, 50, 60))
            };
            let sep = if i > 0 {
                vec![Span::raw("  ")]
            } else {
                vec![]
            };
            let mut out = sep;
            out.push(Span::styled(format!(" {action} "), style));
            out
        })
        .collect();

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_help_bar(f: &mut Frame, app: &App, area: Rect) {
    let bindings = if app.modal.is_some() {
        vec![
            ("Esc", "Close"),
            ("Enter", "Confirm"),
        ]
    } else if app.search_active {
        vec![
            ("Esc", "Cancel"),
            ("Enter", "Confirm"),
            ("type", "Filter"),
        ]
    } else {
        vec![
            ("j/k", "Nav"),
            ("Space", "Activate"),
            ("x", "Mark"),
            ("X", "Mark All"),
            ("e", "Export"),
            ("/", "Search"),
            ("?", "Help"),
            ("h/l", "Host"),
            ("q", "Quit"),
        ]
    };

    let spans: Vec<Span> = bindings
        .iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(format!(" {key} "), Style::default().fg(Color::Black).bg(DIM)),
                Span::styled(format!(" {desc} "), Style::default().fg(DIM)),
                Span::raw(" "),
            ]
        })
        .collect();

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_notice(f: &mut Frame, notice: &crate::app::OperationNotice) {
    let area = f.area();
    let width = (area.width.saturating_sub(4)).min(80);
    let x = (area.width.saturating_sub(width)) / 2;
    let popup = Rect::new(x, area.height.saturating_sub(5), width, 3);

    let (border_color, text_color) = match notice.kind {
        NoticeKind::Success => (ACTIVE_COLOR, ACTIVE_COLOR),
        NoticeKind::Error => (ERROR_COLOR, ERROR_COLOR),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(SURFACE));

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(Span::styled(
            &notice.message,
            Style::default().fg(text_color),
        ))
        .block(block)
        .wrap(Wrap { trim: true }),
        popup,
    );
}

fn draw_delete_modal(f: &mut Frame, app: &App) {
    let area = centered_rect(50, 7, f.area());
    f.render_widget(Clear, area);

    let label = app
        .pending_delete_profile
        .as_ref()
        .and_then(|id| app.selected_host_profiles.iter().find(|p| p.id == *id))
        .map(|p| p.label.as_str())
        .unwrap_or("?");

    let block = Block::default()
        .title(" Confirm Delete ")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(ERROR_COLOR))
        .style(Style::default().bg(SURFACE))
        .padding(Padding::new(2, 2, 1, 0));

    let text = vec![
        Line::from(vec![
            Span::raw("Delete profile "),
            Span::styled(label, Style::default().fg(Color::White).bold()),
            Span::raw("?"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Black).bg(ERROR_COLOR).bold()),
            Span::raw(" Confirm  "),
            Span::styled(" Esc ", Style::default().fg(Color::Black).bg(DIM).bold()),
            Span::raw(" Cancel"),
        ]),
    ];

    f.render_widget(Paragraph::new(text).block(block), area);
}



fn draw_import_modal(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 40, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Import Account ")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(SURFACE))
        .padding(Padding::new(2, 2, 1, 0));

    let text = vec![
        Line::from("Paste account JSON, then press Enter to import."),
        Line::from(""),
        Line::from(Span::styled(
            if app.import_text.is_empty() {
                "(paste not yet supported in TUI — use CLI import)"
            } else {
                &app.import_text
            },
            Style::default().fg(DIM),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Black).bg(ACCENT).bold()),
            Span::raw(" Import  "),
            Span::styled(" Esc ", Style::default().fg(Color::Black).bg(DIM).bold()),
            Span::raw(" Cancel"),
        ]),
    ];

    f.render_widget(Paragraph::new(text).block(block), area);
}

fn draw_help_modal(f: &mut Frame) {
    let area = centered_rect(70, 80, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Keybindings ")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(SURFACE))
        .padding(Padding::new(2, 2, 1, 1));

    let cmds: Vec<(&str, &str)> = vec![
        ("j / k / ↑ / ↓", "Navigate profiles"),
        ("Space", "Activate selected profile on current host"),
        ("x", "Toggle mark on current profile"),
        ("X", "Mark/unmark all visible profiles"),
        ("e", "Export marked profiles (or selected if none marked)"),
        ("Enter", "Focus action bar for selected profile"),
        ("Tab", "Toggle focus between profile list and action bar"),
        ("h / l / ← / →", "Switch between hosts"),
        ("H", "Open host selector popup"),
        ("/", "Search/filter profiles by name"),
        ("Esc", "Clear search filter / close modal"),
        ("", ""),
        ("b", "Login via browser (OAuth PKCE flow)"),
        ("d", "Start device code login"),
        ("D", "Finish pending device code login"),
        ("i", "Import account(s) from JSON or JSON array"),
        ("", ""),
        ("r", "Refresh usage limits for all profiles"),
        ("R", "Reload accounts from disk"),
        ("s", "Sync managed accounts to selected remote host"),
        ("", ""),
        ("u", "Update (when available)"),
        ("?", "Show this help"),
        ("q", "Quit"),
        ("Ctrl+C", "Force quit"),
    ];

    let lines: Vec<Line> = cmds
        .iter()
        .map(|(key, desc)| {
            if key.is_empty() {
                Line::from("")
            } else {
                Line::from(vec![
                    Span::styled(format!("{key:>18}"), Style::default().fg(ACCENT).bold()),
                    Span::styled("  ", Style::default().fg(DIM)),
                    Span::styled(*desc, Style::default().fg(Color::White)),
                ])
            }
        })
        .collect();

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_host_selector(f: &mut Frame, app: &App) {
    let height = (app.hosts.len() as u16 + 2).min(12);
    let area = Rect::new(1, 3, 30, height);
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Select Host ")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(SURFACE));

    let items: Vec<ListItem> = app
        .hosts
        .iter()
        .enumerate()
        .map(|(i, host)| {
            let label = match &host.target {
                domain::HostTarget::Local { .. } => format!("{} *L", host.label),
                domain::HostTarget::Remote(r) => format!("{} *R", r.ssh_alias),
            };
            let style = if i == app.selected_host_index {
                Style::default().fg(Color::Black).bg(ACCENT).bold()
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Span::styled(format!("  {label}"), style))
        })
        .collect();

    f.render_widget(List::new(items).block(block), area);
}

fn draw_update_badge(f: &mut Frame, app: &App) {
    let Some(release) = &app.available_update else {
        return;
    };
    let area = f.area();
    let text = format!(" Update {} available (u) ", release.tag);
    let width = text.len() as u16 + 2;
    let badge_area = Rect::new(
        area.width.saturating_sub(width + 1),
        area.height.saturating_sub(3),
        width,
        3,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(SURFACE));

    f.render_widget(Clear, badge_area);
    f.render_widget(
        Paragraph::new(Span::styled(text, Style::default().fg(Color::Yellow).bold()))
            .block(block),
        badge_area,
    );
}

fn draw_update_modal(f: &mut Frame, app: &App) {
    let area = centered_rect(55, 30, f.area());
    f.render_widget(Clear, area);

    let release = app
        .available_update
        .as_ref()
        .map(|r| (r.tag.as_str(), r.name.as_str()))
        .unwrap_or(("?", ""));

    let block = Block::default()
        .title(" Update Available ")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(SURFACE))
        .padding(Padding::new(2, 2, 1, 0));

    let mut lines = vec![
        Line::from(vec![
            Span::styled("New version: ", Style::default().fg(DIM)),
            Span::styled(release.0, Style::default().fg(Color::Yellow).bold()),
        ]),
    ];
    if !release.1.is_empty() {
        lines.push(Line::from(Span::styled(release.1, Style::default().fg(Color::White))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Current: ", Style::default().fg(DIM)),
        Span::styled(
            crate::updater::current_version(),
            Style::default().fg(Color::White),
        ),
    ]));
    lines.push(Line::from(""));

    if app.update_in_progress {
        lines.push(Line::from(Span::styled(
            "Downloading...",
            Style::default().fg(Color::Yellow).bold(),
        )));
    } else {
        lines.push(Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Black).bg(Color::Yellow).bold()),
            Span::raw(" Update  "),
            Span::styled(" Esc ", Style::default().fg(Color::Black).bg(DIM).bold()),
            Span::raw(" Cancel"),
        ]));
    }

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn format_reset(resets_at: Option<chrono::DateTime<Utc>>) -> String {
    let Some(resets_at) = resets_at else {
        return "reset unknown".into();
    };
    let now = Utc::now();
    let minutes = resets_at.signed_duration_since(now).num_minutes().max(0);
    let hours = minutes as f64 / 60.0;
    if hours >= 24.0 {
        format!("{hours:.1}h | {}", resets_at.format("%d-%m"))
    } else {
        format!("{hours:.1}h | {}", resets_at.format("%H:%M"))
    }
}
