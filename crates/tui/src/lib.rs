pub mod app;
pub mod ui;

use std::io;

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use xedit_host::Desk;

pub fn run(desk: Desk) -> io::Result<()> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;

    let mut term = Terminal::new(CrosstermBackend::new(out))?;
    let mut app = app::App::new(desk);
    let result = app.run(&mut term);

    // the terminal has to go back even when the app bailed, or the shell is left broken
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    result
}
