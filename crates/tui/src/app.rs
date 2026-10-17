use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{backend::CrosstermBackend, Terminal};

use xedit_core::Region;
use xedit_engine::process::Proc;
use xedit_engine::scan::ValueType;
use xedit_host::desk::{Desk, Reply};
use xedit_xtl::request::Request;
use xedit_xtl::Hit;

use crate::ui;

pub enum Mode {
    Normal,
    Filter,
    Scan,
    Refine,
    Edit,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    List,
    Right,
}

#[derive(Clone, Copy, PartialEq)]
pub enum RightView {
    Regions,
    Matches,
}

const PAGE: isize = 10;

/// How often live polling wakes up, and how long we sleep when it is off.
const LIVE: Duration = Duration::from_millis(200);
const IDLE: Duration = Duration::from_secs(3600);

/// A frozen value should not be seen wrong for long, so this ticks faster.
const FREEZE: Duration = Duration::from_millis(80);

/// Above this many matches, polling them all is not worth it and we stop.
const LIVE_MAX: usize = 100_000;

/// An address held at a fixed value. Re-written on every tick so the program
/// cannot drift it.
struct Pin {
    addr: u64,
    kind: ValueType,
    bits: u64,
}

pub struct App {
    desk: Desk,
    procs: Vec<Proc>,
    filter: String,
    selected: usize,

    regions: Vec<Region>,
    region_sel: usize,

    hits: Vec<Hit>,
    live_vals: Vec<u64>,
    freeze: Vec<Pin>,
    match_sel: usize,
    kind: Option<ValueType>,
    edit_idx: Option<usize>,

    focus: Focus,
    right: RightView,
    mode: Mode,
    input: String,
    help: bool,
    live: bool,
    status: String,
    target: Option<u32>,
}

impl App {
    pub fn new(desk: Desk) -> Self {
        App {
            desk,
            procs: Vec::new(),
            filter: String::new(),
            selected: 0,
            regions: Vec::new(),
            region_sel: 0,
            hits: Vec::new(),
            live_vals: Vec::new(),
            freeze: Vec::new(),
            match_sel: 0,
            kind: None,
            edit_idx: None,
            focus: Focus::List,
            right: RightView::Regions,
            mode: Mode::Normal,
            input: String::new(),
            help: false,
            live: false,
            status: String::new(),
            target: None,
        }
    }

    pub fn run(&mut self, term: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        self.reload();

        loop {
            term.draw(|frame| ui::draw(frame, self))?;

            // when live polling is on this wakes on a timer, otherwise it
            // blocks until a key and costs nothing
            let wait = if !self.freeze.is_empty() {
                FREEZE
            } else if self.live {
                LIVE
            } else {
                IDLE
            };
            if event::poll(wait)? {
                let Event::Key(key) = event::read()? else {
                    continue;
                };
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if self.handle(key) {
                    break;
                }
            } else {
                self.tick();
            }
        }
        Ok(())
    }

    fn tick(&mut self) {
        self.pin_frozen();

        if !self.live {
            return;
        }
        if self.kind.is_some() && !self.hits.is_empty() {
            self.poll_hits();
        } else {
            self.quiet_reload();
        }
    }

    fn pin_frozen(&mut self) {
        if self.freeze.is_empty() {
            return;
        }

        let pins: Vec<(u64, Vec<u8>)> = self
            .freeze
            .iter()
            .map(|p| (p.addr, p.kind.encode(p.bits)))
            .collect();

        for (addr, bytes) in pins {
            let _ = self.desk.handle(Request::Pin { addr, bytes });
        }
    }

    fn poll_hits(&mut self) {
        let Some(kind) = self.kind else {
            return;
        };
        if self.hits.len() > LIVE_MAX {
            self.live = false;
            self.status = String::from("live off, too many matches to poll");
            return;
        }

        let req = Request::Refresh {
            kind: kind.name().to_string(),
            hits: self.hits.clone(),
        };
        match self.desk.handle(req) {
            // only the shown values move. hits keeps the scan-time numbers so
            // refine (changed, increased) still has something to compare against
            Ok(Reply::Matches(now)) if now.len() == self.hits.len() => {
                self.live_vals = now.into_iter().map(|h| h.prev).collect();
            }
            Ok(_) => {}
            Err(e) => {
                self.live = false;
                self.status = format!("live off: {e}");
            }
        }
    }

    // list refresh that keeps the cursor where it was and leaves the status alone
    fn quiet_reload(&mut self) {
        let keep = self.visible().nth(self.selected).map(|p| p.pid);

        if let Ok(Reply::Processes(mut procs)) = self.desk.handle(Request::ListProcesses) {
            let me = std::process::id();
            procs.retain(|p| p.pid != me);
            self.procs = procs;
        }

        if let Some(pid) = keep {
            let found = self.visible().position(|p| p.pid == pid);
            if let Some(i) = found {
                self.selected = i;
            }
        }

        let shown = self.visible().count();
        if self.selected >= shown {
            self.selected = shown.saturating_sub(1);
        }
    }

    fn handle(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }

        // the help card swallows input until it is dismissed
        if self.help {
            self.help = false;
            return false;
        }
        if key.code == KeyCode::Char('?') {
            self.help = true;
            return false;
        }

        match self.mode {
            Mode::Filter => return self.key_filter(key),
            Mode::Scan | Mode::Refine | Mode::Edit => return self.key_input(key),
            Mode::Normal => {}
        }

        if key.code == KeyCode::Char('l') {
            self.live = !self.live;
            self.status = if self.live {
                String::from("live on")
            } else {
                String::from("live off")
            };
            return false;
        }

        if key.code == KeyCode::Tab {
            self.focus = match self.focus {
                Focus::List => Focus::Right,
                Focus::Right => Focus::List,
            };
            return false;
        }

        match self.focus {
            Focus::List => self.key_list(key),
            Focus::Right => self.key_right(key),
        }
    }

    fn key_list(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = bump(self.selected, -1, self.visible().count())
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = bump(self.selected, 1, self.visible().count())
            }
            KeyCode::PageUp => self.selected = bump(self.selected, -PAGE, self.visible().count()),
            KeyCode::PageDown => self.selected = bump(self.selected, PAGE, self.visible().count()),
            KeyCode::Char('g') => self.selected = 0,
            KeyCode::Char('G') => self.selected = self.visible().count().saturating_sub(1),
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Char('r') => self.reload(),
            KeyCode::Char('s') => self.begin(Mode::Scan),
            KeyCode::Enter => self.open(),
            _ => {}
        }
        false
    }

    fn key_right(&mut self, key: KeyEvent) -> bool {
        match key.code {
            // step out of the pane, back to the process list
            KeyCode::Char('q') | KeyCode::Esc => self.focus = Focus::List,
            KeyCode::Up | KeyCode::Char('k') => self.step_right(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step_right(1),
            KeyCode::PageUp => self.step_right(-PAGE),
            KeyCode::PageDown => self.step_right(PAGE),
            KeyCode::Char('g') => self.top_right(),
            KeyCode::Char('G') => self.bottom_right(),
            KeyCode::Char('v') => {
                self.right = match self.right {
                    RightView::Regions => RightView::Matches,
                    RightView::Matches => RightView::Regions,
                };
            }
            KeyCode::Char('s') => self.begin(Mode::Scan),
            KeyCode::Char('n') => self.begin(Mode::Refine),
            KeyCode::Char('e') if self.right == RightView::Matches => self.begin_edit(),
            KeyCode::Char('f') if self.right == RightView::Matches => self.toggle_freeze(),
            KeyCode::Char('c') => self.clear_matches(),
            _ => {}
        }
        false
    }

    fn key_filter(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.selected = 0;
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.selected = 0;
            }
            _ => {}
        }
        false
    }

    fn key_input(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.input.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
        false
    }

    fn begin(&mut self, mode: Mode) {
        self.input.clear();
        self.mode = mode;
    }

    fn submit(&mut self) {
        let line = self.input.trim().to_string();
        self.input.clear();

        if matches!(self.mode, Mode::Scan) {
            self.mode = Mode::Normal;
            self.run_scan(&line);
        } else if matches!(self.mode, Mode::Refine) {
            self.mode = Mode::Normal;
            self.run_refine(&line);
        } else if matches!(self.mode, Mode::Edit) {
            self.mode = Mode::Normal;
            self.run_edit(&line);
        }
    }

    fn run_scan(&mut self, line: &str) {
        if self.target.is_none() {
            self.status = String::from("open a process first");
            return;
        }

        let mut parts = line.split_whitespace();
        let (Some(kind), Some(value)) = (parts.next(), parts.next()) else {
            self.status = String::from("scan wants a type and a value, like i32 42");
            return;
        };
        let Some(vt) = ValueType::parse(kind) else {
            self.status = format!("unknown type {kind}");
            return;
        };

        match self.desk.handle(Request::Scan {
            kind: kind.to_string(),
            value: value.to_string(),
            max: None,
        }) {
            Ok(Reply::Matches(hits)) => {
                self.status = format!("{} matches", hits.len());
                self.hits = hits;
                self.live_vals.clear();
                self.match_sel = 0;
                self.kind = Some(vt);
                self.right = RightView::Matches;
                self.focus = Focus::Right;
            }
            Ok(_) => {}
            Err(e) => self.status = format!("scan: {e}"),
        }
    }

    fn run_refine(&mut self, line: &str) {
        let Some(vt) = self.kind else {
            self.status = String::from("nothing to refine, scan first");
            return;
        };
        if self.hits.is_empty() {
            self.status = String::from("no matches left");
            return;
        }

        let mut parts = line.split_whitespace();
        let Some(filter) = parts.next() else {
            self.status =
                String::from("refine: changed, unchanged, increased, decreased or exact <value>");
            return;
        };
        let value = parts.next().unwrap_or("");

        let before = self.hits.len();
        match self.desk.handle(Request::Refine {
            kind: vt.name().to_string(),
            filter: filter.to_string(),
            value: value.to_string(),
            hits: self.hits.clone(),
        }) {
            Ok(Reply::Matches(hits)) => {
                self.status = format!("{filter}: {before} -> {}", hits.len());
                self.hits = hits;
                self.live_vals.clear();
                self.match_sel = 0;
                self.right = RightView::Matches;
                self.focus = Focus::Right;
            }
            Ok(_) => {}
            Err(e) => self.status = format!("refine: {e}"),
        }
    }

    fn clear_matches(&mut self) {
        self.hits.clear();
        self.live_vals.clear();
        self.freeze.clear();
        self.edit_idx = None;
        self.match_sel = 0;
        self.kind = None;
        self.status = String::from("cleared");
    }

    fn begin_edit(&mut self) {
        if self.kind.is_none() || self.hits.is_empty() {
            return;
        }

        self.edit_idx = Some(self.match_sel);
        self.input.clear();
        self.mode = Mode::Edit;
    }

    fn run_edit(&mut self, text: &str) {
        let (Some(kind), Some(i)) = (self.kind, self.edit_idx) else {
            self.status = String::from("nothing to edit");
            return;
        };
        self.edit_idx = None;

        let Some(bits) = kind.parse_value(text) else {
            self.status = format!("not a {}", kind.name());
            return;
        };

        let addr = self.hits[i].addr;
        match self.desk.handle(Request::Write {
            addr,
            bytes: kind.encode(bits),
        }) {
            Ok(Reply::Wrote { addr, .. }) => {
                self.status = format!("wrote {} at 0x{:x}", kind.format(bits), addr.0);
                self.hits[i].prev = bits;
                if i < self.live_vals.len() {
                    self.live_vals[i] = bits;
                }
            }
            Ok(_) => {}
            Err(e) => self.status = format!("write: {e}"),
        }
    }

    fn toggle_freeze(&mut self) {
        if self.kind.is_none() || self.hits.is_empty() {
            return;
        }

        let i = self.match_sel;
        let addr = self.hits[i].addr;

        if let Some(pos) = self.freeze.iter().position(|p| p.addr == addr) {
            self.freeze.remove(pos);
            self.status = format!("unfroze 0x{addr:x}");
            return;
        }

        let kind = self.kind.unwrap();
        let bits = self.value_at(i);
        self.freeze.push(Pin { addr, kind, bits });
        self.status = format!("froze 0x{addr:x} at {}", kind.format(bits));
    }

    fn open(&mut self) {
        let Some(pid) = self.visible().nth(self.selected).map(|p| p.pid) else {
            return;
        };

        match self.desk.handle(Request::Attach { pid }) {
            Ok(_) => {
                self.target = Some(pid);
                self.status = format!("attached to {pid}");
                self.hits.clear();
                self.live_vals.clear();
                self.freeze.clear();
                self.kind = None;
                self.load_regions(pid);
                self.right = RightView::Regions;
                self.focus = Focus::Right;
            }
            Err(e) => self.status = format!("attach {pid}: {e}"),
        }
    }

    fn load_regions(&mut self, pid: u32) {
        match self.desk.handle(Request::Regions { pid }) {
            Ok(Reply::Regions(regions)) => {
                self.regions = regions;
                self.region_sel = 0;
            }
            Ok(_) => {}
            Err(e) => {
                self.regions.clear();
                self.status = format!("regions {pid}: {e}");
            }
        }
    }

    fn reload(&mut self) {
        match self.desk.handle(Request::ListProcesses) {
            Ok(Reply::Processes(mut procs)) => {
                let me = std::process::id();
                procs.retain(|p| p.pid != me);
                self.status = format!("{} processes", procs.len());
                self.procs = procs;
            }
            Ok(_) => {}
            Err(e) => self.status = format!("list: {e}"),
        }

        let shown = self.visible().count();
        if self.selected >= shown {
            self.selected = shown.saturating_sub(1);
        }
    }

    fn step_right(&mut self, delta: isize) {
        match self.right {
            RightView::Regions => {
                self.region_sel = bump(self.region_sel, delta, self.regions.len())
            }
            RightView::Matches => self.match_sel = bump(self.match_sel, delta, self.hits.len()),
        }
    }

    fn top_right(&mut self) {
        match self.right {
            RightView::Regions => self.region_sel = 0,
            RightView::Matches => self.match_sel = 0,
        }
    }

    fn bottom_right(&mut self) {
        match self.right {
            RightView::Regions => self.region_sel = self.regions.len().saturating_sub(1),
            RightView::Matches => self.match_sel = self.hits.len().saturating_sub(1),
        }
    }

    pub fn visible(&self) -> impl Iterator<Item = &Proc> {
        let needle = self.filter.trim().to_lowercase();
        self.procs
            .iter()
            .filter(move |p| needle.is_empty() || p.matches(&needle))
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn total(&self) -> usize {
        self.procs.len()
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    pub fn region_sel(&self) -> usize {
        self.region_sel
    }

    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    pub fn match_sel(&self) -> usize {
        self.match_sel
    }

    pub fn kind(&self) -> Option<ValueType> {
        self.kind
    }

    /// What to show for match `i`: the live value if polling has one, else the
    /// value it had when the scan or last refine ran.
    pub fn value_at(&self, i: usize) -> u64 {
        self.live_vals.get(i).copied().unwrap_or(self.hits[i].prev)
    }

    pub fn is_frozen(&self, addr: u64) -> bool {
        self.freeze.iter().any(|p| p.addr == addr)
    }

    pub fn edit_value(&self) -> Option<u64> {
        self.edit_idx.map(|i| self.value_at(i))
    }

    pub fn frozen_count(&self) -> usize {
        self.freeze.len()
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn right(&self) -> RightView {
        self.right
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn help(&self) -> bool {
        self.help
    }

    pub fn live(&self) -> bool {
        self.live
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn target(&self) -> Option<u32> {
        self.target
    }
}

fn bump(cur: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (cur as isize + delta).clamp(0, len as isize - 1) as usize
}
