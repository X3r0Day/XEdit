use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{App, Focus, Mode, RightView};

const HELP: &str = "\
xedit

 process list
   up / down      move             g / G    top / bottom
   enter          open process     pgup/pg  page
   /              find             s        scan
   r              reload
   tab            go to the right pane
   q / esc        quit

 right pane
   up / down      move
   v              regions <-> matches
   s              scan
   n              refine the matches
   e              edit (poke) the selected value
   f              freeze it, held at the value you set
   c              clear the matches and freezes
   q / esc        back to the list
   tab            go to the list

 scans
   first scan     <kind> <value>      e.g.  i32 42
   refine         changed | unchanged | increased | decreased | exact <value>

 anywhere
   ?              this help
   l              live polling on/off
   ctrl-c         quit";

pub fn draw(f: &mut Frame, app: &App) {
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(f.area());
    let panes =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(rows[0]);

    draw_procs(f, app, panes[0]);
    draw_right(f, app, panes[1]);
    f.render_widget(Paragraph::new(footer(app)), rows[1]);

    if app.help() {
        let box_area = centered(f.area(), 64, 25);
        f.render_widget(Clear, box_area);
        let help =
            Paragraph::new(HELP).block(Block::default().borders(Borders::ALL).title(" help "));
        f.render_widget(help, box_area);
    }
}

fn footer(app: &App) -> String {
    match app.mode() {
        Mode::Filter => format!("find process: {}_    enter keep   esc clear", app.filter()),
        Mode::Scan => format!(
            "scan  <kind> <value>  e.g. i32 42 : {}_    enter run   esc cancel",
            app.input()
        ),
        Mode::Refine => format!(
            "refine  changed / unchanged / increased / decreased / exact <value> : {}_    enter run   esc cancel",
            app.input()
        ),
        Mode::Edit => {
            let now = app
                .edit_value()
                .map_or(String::from("?"), |v| app.kind().map_or(v.to_string(), |k| k.format(v)));
            format!(
                "edit  <{}>  now {now} : {}_    enter write   esc cancel",
                app.kind().map_or("?", |k| k.name()),
                app.input()
            )
        }
        Mode::Normal => match app.focus() {
            Focus::List => format!(
                "focus: list  live:{}  |  {}  |  \u{2191}\u{2193} move  enter open  s scan  / find  r reload  l poll  tab right  ? help  q quit",
                on_off(app.live()),
                app.status()
            ),
            Focus::Right => {
                let pane = match app.right() {
                    RightView::Regions => "regions",
                    RightView::Matches => "matches",
                };
                let keys = match app.right() {
                    RightView::Regions => "v matches  s scan  q/esc back",
                    RightView::Matches => {
                        "n refine  e edit  f freeze  s scan  c clear  v regions  q/esc back"
                    }
                };
                format!(
                    "focus: right [{pane}]  live:{}  |  {}  |  \u{2191}\u{2193} move  {keys}  l poll  tab list  ? help",
                    on_off(app.live()),
                    app.status()
                )
            }
        },
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

fn draw_procs(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .visible()
        .map(|p| ListItem::new(p.to_string()))
        .collect();
    let title = format!(" processes {}/{} ", items.len(), app.total());

    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.selected()));
    }

    let list = List::new(items)
        .block(pane(title, app.focus() == Focus::List))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, area, &mut state);
}

fn draw_right(f: &mut Frame, app: &App, area: Rect) {
    match app.right() {
        RightView::Regions => draw_regions(f, app, area),
        RightView::Matches => draw_matches(f, app, area),
    }
}

fn draw_regions(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus() == Focus::Right;

    let Some(pid) = app.target() else {
        let hint = Paragraph::new("\n  pick a process, enter opens it")
            .block(pane(String::from(" regions "), focused));
        f.render_widget(hint, area);
        return;
    };

    let items: Vec<ListItem> = app
        .regions()
        .iter()
        .map(|r| ListItem::new(r.to_string()))
        .collect();
    let title = format!(" regions {pid} ({}) ", items.len());

    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.region_sel()));
    }

    let list = List::new(items)
        .block(pane(title, focused))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, area, &mut state);
}

fn draw_matches(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus() == Focus::Right;
    let kind = app
        .kind()
        .map_or(String::from("?"), |k| k.name().to_string());
    let kept = app.frozen_count();
    let title = if kept > 0 {
        format!(" matches {kind} ({}) - {kept} kept ", app.hits().len())
    } else {
        format!(" matches {kind} ({}) ", app.hits().len())
    };

    if app.target().is_none() {
        let hint = Paragraph::new("\n  pick a process, enter opens it").block(pane(title, focused));
        f.render_widget(hint, area);
        return;
    }
    if app.hits().is_empty() {
        let hint =
            Paragraph::new("\n  press s and type a value to scan for").block(pane(title, focused));
        f.render_widget(hint, area);
        return;
    }

    let items: Vec<ListItem> = app
        .hits()
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let raw = app.value_at(i);
            let value = app.kind().map_or(raw.to_string(), |k| k.format(raw));
            let held = if app.is_frozen(h.addr) {
                "   [kept]"
            } else {
                ""
            };
            ListItem::new(format!("0x{:012x}  {value}{held}", h.addr))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.match_sel()));

    let list = List::new(items)
        .block(pane(title, focused))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, area, &mut state);
}

fn pane(title: String, focused: bool) -> Block<'static> {
    let border = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(title)
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let width = w.min(area.width);
    let height = h.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
