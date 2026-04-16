mod app;
mod ui;

use app::{App, FocusArea, Modal};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
    let mut app = App::new()?;

    loop {
        app.poll_background();
        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
                {
                    break;
                }

                // Search mode input
                if app.search_active {
                    match key.code {
                        KeyCode::Esc => app.cancel_search(),
                        KeyCode::Enter => app.finish_search(),
                        KeyCode::Backspace => app.search_pop(),
                        KeyCode::Char(c) => app.search_push(c),
                        _ => {}
                    }
                    continue;
                }

                // Modal input
                if let Some(modal) = app.modal {
                    match key.code {
                        KeyCode::Esc => app.close_modal(),
                        KeyCode::Enter => match modal {
                            Modal::DeleteConfirm => app.confirm_delete(),
                            Modal::Import => app.import_from_text(),
                            Modal::Help => app.close_modal(),
                        },
                        _ => {}
                    }
                    continue;
                }

                // Host selector
                if app.host_selector_open {
                    match key.code {
                        KeyCode::Esc => app.host_selector_open = false,
                        KeyCode::Char('j') | KeyCode::Down => app.next_host(),
                        KeyCode::Char('k') | KeyCode::Up => app.prev_host(),
                        KeyCode::Enter => app.host_selector_open = false,
                        _ => {}
                    }
                    continue;
                }

                // Global keys
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('/') => app.start_search(),
                    KeyCode::Char('?') => app.modal = Some(Modal::Help),
                    KeyCode::Char('b') => app.login_browser(),
                    KeyCode::Char('d') => app.start_device_login(),
                    KeyCode::Char('D') => app.finish_device_login(),
                    KeyCode::Char('i') => app.open_import_modal(),
                    KeyCode::Char('r') => app.refresh_all_limits(),
                    KeyCode::Char('R') => app.reload_accounts(),
                    KeyCode::Char('s') if app.selected_host_is_remote() => {
                        app.sync_selected_remote()
                    }
                    KeyCode::Char('h') | KeyCode::Left => app.prev_host(),
                    KeyCode::Char('l') | KeyCode::Right => app.next_host(),
                    KeyCode::Char('H') => {
                        app.host_selector_open = true;
                        app.focus = FocusArea::HostSelector;
                    }
                    KeyCode::Esc => {
                        if !app.search_query.is_empty() {
                            app.cancel_search();
                        } else if !app.marked_profiles.is_empty() {
                            app.clear_marks();
                        }
                    }
                    KeyCode::Tab => {
                        app.focus = match app.focus {
                            FocusArea::ProfileList => FocusArea::ActionBar,
                            FocusArea::ActionBar => FocusArea::ProfileList,
                            FocusArea::HostSelector => FocusArea::ProfileList,
                        };
                    }
                    KeyCode::BackTab => {
                        app.focus = match app.focus {
                            FocusArea::ProfileList => FocusArea::ActionBar,
                            FocusArea::ActionBar => FocusArea::ProfileList,
                            FocusArea::HostSelector => FocusArea::ProfileList,
                        };
                    }
                    _ => {}
                }

                // Focus-specific keys
                match app.focus {
                    FocusArea::ProfileList => match key.code {
                        KeyCode::Char('j') | KeyCode::Down => app.next_profile(),
                        KeyCode::Char('k') | KeyCode::Up => app.prev_profile(),
                        KeyCode::Char(' ') => app.activate_selected_profile(),
                        KeyCode::Char('x') => {
                            app.toggle_mark();
                            app.next_profile();
                        }
                        KeyCode::Char('X') => app.mark_all_visible(),
                        KeyCode::Char('e') => app.export_selected_profile(),
                        KeyCode::Enter => {
                            app.focus = FocusArea::ActionBar;
                            app.action_bar_index = 0;
                        }
                        _ => {}
                    },
                    FocusArea::ActionBar => match key.code {
                        KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('k') | KeyCode::Up => {
                            app.focus = FocusArea::ProfileList;
                        }
                        KeyCode::Left => {
                            app.action_bar_index = app.action_bar_index.saturating_sub(1);
                        }
                        KeyCode::Right => {
                            app.action_bar_index = (app.action_bar_index + 1).min(2);
                        }
                        KeyCode::Enter => match app.action_bar_index {
                            0 => app.activate_selected_profile(),
                            1 => app.export_selected_profile(),
                            2 => app.delete_selected_profile(),
                            _ => {}
                        },
                        _ => {}
                    },
                    FocusArea::HostSelector => {}
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
