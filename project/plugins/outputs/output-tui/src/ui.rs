use crate::client::{After, Call, Snapshot};
use anyhow::{bail, Context, Result};
use clap::Parser;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::Rect;
use serde_json::{json, Value};
use tokio::sync::mpsc;

pub const HELP:&str="TERMINAL WORKBENCH\n\nTab / Shift-Tab   select next / previous panel\np                  pages: select, create, clone, delete\ns                  all streams: inspect and add log panel\na / c              add log / curve panel\ne / E              panel / page inspector\ny                  series: add, edit, remove, legend selection\n:                  full CLI command palette (named parameters)\n/ / Ctrl-R         text / regular-expression filter\nSpace / f          pause / follow newest rows\nt / i / g          hex / metadata / legend\nh / l / Ctrl-F     history / live / full archive search\n[ / ]              previous / next fixed history page\nUp / Down          select log row; Enter locates its context\nI                  selected row metadata and byte identity\no                  actual archive coverage and query progress\nEsc                dismiss or cancel a layout draft\nm / r              move / resize selected panel with arrows\nEnter              commit move / resize in one revision\nMouse              click, drag title, resize lower-right corner\nAlt-arrows         pan the canvas\n+ / - / 0          canvas zoom / fit all panels\nCtrl-+ / Ctrl--    zoom the selected curve time interval\nb / e / z          sidebar / inspector / minimap\nL / Delete         lock position / confirm panel removal\nq / Ctrl-C         detach (collection continues)\n\nCOMMAND PALETTE EXAMPLES\npage create --name monitor --title 'My monitor'\npage set --theme dark --layout-mode canvas\npanel set --left 24 --top 24 --panel-width 640 --panel-height 320\npanel set --stream source --channel stderr --format hex\npanel set --column-width time=124 --column text --column time\nseries add --name Temperature --regex 'temperature=(?P<value>[0-9.]+)'\nseries set --series UUID --color '#ffb454' --width 2\nhistory search --regex ERROR\nquery cancel --query UUID\n\nPage/panel selectors default to the current selection. Mutations use\nthe revision captured when editing began; conflicts never overwrite.\nPhysical font size is controlled by the terminal. Canvas units remain\npixels (8 px per column / 16 px per row at 100%); use zoom to fit.\n";

#[derive(Parser)]
#[command(name = "", disable_help_subcommand = true)]
struct Palette {
    #[command(subcommand)]
    command: log_view::cli::ViewAction,
}
#[derive(Clone)]
pub enum PromptKind {
    Command,
    Filter,
    Regex,
    PageName,
    Field {
        page: bool,
        key: String,
        kind: FieldType,
    },
}
#[derive(Clone, Copy)]
pub enum FieldType {
    Text,
    Number,
    Bool,
    List,
}
#[derive(Clone)]
pub struct Prompt {
    pub title: String,
    pub text: String,
    pub cursor: usize,
    pub kind: PromptKind,
    pub revision: u64,
    pub page: String,
    pub panel: Option<String>,
}
#[derive(Clone, Copy, PartialEq)]
pub enum Menu {
    Pages,
    Streams,
    Series,
    Panel,
    Page,
}
#[derive(Clone)]
pub enum Modal {
    Prompt(Prompt),
    Menu {
        kind: Menu,
        index: usize,
    },
    Info {
        title: String,
        text: String,
        offset: u16,
    },
    Confirm {
        method: String,
        args: Value,
        title: String,
    },
    Result,
}
#[derive(Clone)]
pub enum HitKind {
    Page(String),
    Panel(String),
    Header(String),
    Resize(String),
    Row(String, usize),
    Source(usize),
}
#[derive(Clone)]
pub struct Hit {
    pub area: Rect,
    pub kind: HitKind,
}
pub struct Draft {
    pub panel: Value,
    pub original: Value,
    pub revision: u64,
    pub start: (u16, u16),
    pub resize: bool,
    pub mouse: bool,
}
pub struct Pan {
    pub start: (u16, u16),
    pub origin: (f64, f64),
    pub current: (f64, f64),
    pub revision: u64,
}
pub struct App {
    pub snapshot: Snapshot,
    pub modal: Option<Modal>,
    pub hits: Vec<Hit>,
    pub draft: Option<Draft>,
    pub pan: Option<Pan>,
    pub row: usize,
    pub scroll: usize,
    pub horizontal: u16,
    pub status: String,
    pub body: Rect,
    last_panel: String,
}
impl App {
    pub fn new() -> Self {
        Self {
            snapshot: Snapshot::default(),
            modal: None,
            hits: vec![],
            draft: None,
            pan: None,
            row: 0,
            scroll: 0,
            horizontal: 0,
            status: String::new(),
            body: Rect::default(),
            last_panel: String::new(),
        }
    }
    pub fn accept(&mut self, snapshot: Snapshot) {
        if snapshot.state["revision"] != self.snapshot.state["revision"]
            || snapshot.notice != self.snapshot.notice
        {
            self.status.clear();
        }
        self.snapshot = snapshot;
        let id = self
            .selected()
            .and_then(|p| p["id"].as_str())
            .unwrap_or_default()
            .to_string();
        if id != self.last_panel {
            self.row = 0;
            self.scroll = 0;
            self.horizontal = 0;
            self.last_panel = id;
        }
    }
    pub fn revision(&self) -> u64 {
        self.snapshot.state["revision"].as_u64().unwrap_or(0)
    }
    pub fn page(&self) -> Option<&Value> {
        self.snapshot.state["pages"]
            .as_array()?
            .iter()
            .find(|p| p["id"] == self.snapshot.state["selected"])
    }
    pub fn panels(&self) -> Vec<Value> {
        self.page()
            .and_then(|p| p["panels"].as_array())
            .cloned()
            .unwrap_or_default()
    }
    pub fn selected(&self) -> Option<&Value> {
        let p = self.page()?;
        let panels = p["panels"].as_array()?;
        panels
            .iter()
            .find(|v| v["id"] == p["active_panel"])
            .or_else(|| panels.iter().find(|v| v["hidden"] != true))
    }
    pub fn panel_id(&self) -> Option<String> {
        self.selected()?.get("id")?.as_str().map(str::to_owned)
    }
    pub fn page_id(&self) -> String {
        self.page()
            .and_then(|p| p["id"].as_str())
            .unwrap_or_default()
            .to_owned()
    }
    pub fn panel_data(&self, panel: &Value) -> Value {
        self.snapshot
            .panels
            .get(panel["id"].as_str().unwrap_or_default())
            .cloned()
            .unwrap_or_else(|| json!({}))
    }
    fn send(&mut self, tx: &mpsc::Sender<Call>, call: Call) {
        if let Err(e) = tx.try_send(call) {
            self.status = format!("Busy: {e}");
        } else {
            self.status = "Saving…".into();
        }
    }
    fn set(&mut self, tx: &mpsc::Sender<Call>, page: bool, mut args: Value) {
        args["page"] = json!(self.page_id());
        args["revision"] = json!(self.revision());
        if !page {
            let Some(id) = self.panel_id() else {
                return;
            };
            args["panel"] = json!(id);
        }
        self.send(
            tx,
            Call::new(if page { "page.set" } else { "panel.set" }, args),
        );
    }
    fn prompt(&mut self, title: &str, text: String, kind: PromptKind) {
        self.modal = Some(Modal::Prompt(Prompt {
            title: title.into(),
            cursor: text.chars().count(),
            text,
            kind,
            revision: self.revision(),
            page: self.page_id(),
            panel: self.panel_id(),
        }));
    }
    pub fn event(&mut self, event: Event, tx: &mpsc::Sender<Call>) -> Result<bool> {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return Ok(true);
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(false);
            }
            if self.modal.is_some() {
                self.modal_key(key, tx);
                return Ok(true);
            }
            if self.draft.as_ref().is_some_and(|d| !d.mouse) {
                match key.code {
                    KeyCode::Esc => {
                        self.draft = None;
                        self.status = "Layout draft discarded".into();
                    }
                    KeyCode::Enter => self.commit_draft(tx),
                    KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                        let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                            5.
                        } else {
                            1.
                        };
                        let (dx, dy) = match key.code {
                            KeyCode::Left => (-8. * step, 0.),
                            KeyCode::Right => (8. * step, 0.),
                            KeyCode::Up => (0., -16. * step),
                            _ => (0., 16. * step),
                        };
                        self.adjust_draft(dx, dy);
                    }
                    _ => {}
                }
                return Ok(true);
            }
            let panel = self.selected().cloned().unwrap_or_else(|| json!({}));
            let page = self.page().cloned().unwrap_or_else(|| json!({}));
            if key.modifiers.contains(KeyModifiers::ALT)
                && matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
                )
            {
                let (x, y) = match key.code {
                    KeyCode::Left => (32., 0.),
                    KeyCode::Right => (-32., 0.),
                    KeyCode::Up => (0., 64.),
                    _ => (0., -64.),
                };
                self.set(tx,true,json!({"view_x":number(&page,"view_x",0.)+x,"view_y":number(&page,"view_y",0.)+y}));
                return Ok(true);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('r') => self.prompt(
                        "Regular expression",
                        text(&panel, "regex"),
                        PromptKind::Regex,
                    ),
                    KeyCode::Char('f') => self.history(tx, "history.search", None),
                    KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('-') => {
                        let start = number(&panel, "zoom_start", 0.);
                        let end = number(&panel, "zoom_end", 100.);
                        let center = (start + end) / 2.;
                        let half = (end - start)
                            * if key.code == KeyCode::Char('-') {
                                0.625
                            } else {
                                0.4
                            };
                        self.set(tx,false,json!({"zoom_start":(center-half).max(0.),"zoom_end":(center+half).min(100.)}));
                    }
                    _ => {}
                }
                return Ok(true);
            }
            match key.code {
                KeyCode::Char('q')=>return Ok(false),
                KeyCode::Char('?')=>self.modal=Some(Modal::Info{title:"Help / command reference".into(),text:HELP.into(),offset:0}),
                KeyCode::Char(':')=>self.prompt("Command · same parameters as log-print tui ID",String::new(),PromptKind::Command),
                KeyCode::Char('/')=>self.prompt("Text filter",text(&panel,"text"),PromptKind::Filter),
                KeyCode::Char('p')=>self.modal=Some(Modal::Menu{kind:Menu::Pages,index:0}),
                KeyCode::Char('s')=>self.modal=Some(Modal::Menu{kind:Menu::Streams,index:0}),
                KeyCode::Char('y')=>self.modal=Some(Modal::Menu{kind:Menu::Series,index:0}),
                KeyCode::Char('e')=>self.modal=Some(Modal::Menu{kind:Menu::Panel,index:0}),
                KeyCode::Char('E')=>self.modal=Some(Modal::Menu{kind:Menu::Page,index:0}),
                KeyCode::Char('n')=>self.prompt("Create Page",String::new(),PromptKind::PageName),
                KeyCode::Char('a')=>self.add(tx,"log",None),KeyCode::Char('c')=>self.add(tx,"curve",None),
                KeyCode::Char(' ')=>self.set(tx,false,json!({"paused":panel["paused"]!=true})),
                KeyCode::Char('f')=>{self.row=0;self.scroll=0;self.set(tx,false,json!({"follow":panel["follow"]!=true}));},
                KeyCode::Char('t')=>self.set(tx,false,json!({"format":if panel["format"]=="hex"{"text"}else{"hex"}})),
                KeyCode::Char('i')=>self.set(tx,false,json!({"metadata":panel["metadata"]!=true})),
                KeyCode::Char('g')=>self.set(tx,false,json!({"legend":panel["legend"]!=true})),
                KeyCode::Char('b')=>self.set(tx,true,json!({"sidebar_open":page["sidebar_open"]!=true})),
                KeyCode::Char('z')=>self.set(tx,true,json!({"show_minimap":page["show_minimap"]!=true})),
                KeyCode::Char('L')=>self.set(tx,false,json!({"locked":panel["locked"]!=true})),
                KeyCode::Char('h')=>self.history(tx,"history.read",None),
                KeyCode::Char('l')=>self.set(tx,false,json!({"mode":"live","query":null,"offset":0,"paused":false})),
                KeyCode::Char('o')=>self.modal=Some(Modal::Info{title:"Coverage / gaps / query progress".into(),text:serde_json::to_string_pretty(&json!({"archive_enabled":self.snapshot.state["archive_enabled"],"writer":self.snapshot.state["archive_writer"],"coverage":self.panel_data(&panel)["status"]["coverage"],"query_state":self.panel_data(&panel)["status"]["state"],"scanned":self.panel_data(&panel)["status"]["scanned"],"error":self.panel_data(&panel)["error"]})).unwrap_or_default(),offset:0}),
                KeyCode::Char('I')=>{
                    let rows=sorted_rows(&self.panel_data(&panel),&panel);
                    if let Some(row)=rows.get(self.row.min(rows.len().saturating_sub(1))) {self.modal=Some(Modal::Info{title:"Record identity / metadata".into(),text:serde_json::to_string_pretty(row).unwrap_or_default(),offset:0});}
                },
                KeyCode::Tab|KeyCode::BackTab=>self.next_panel(tx,key.code==KeyCode::BackTab),
                KeyCode::Char('m')|KeyCode::Char('r')=>self.begin_draft(panel,(0,0),key.code==KeyCode::Char('r'),false),
                KeyCode::Delete=>if let Some(id)=self.panel_id(){self.modal=Some(Modal::Confirm{method:"panel.remove".into(),args:json!({"page":self.page_id(),"panel":id,"revision":self.revision()}),title:"Remove selected panel? Enter confirms; Esc keeps it.".into()});},
                KeyCode::Char('[')|KeyCode::PageUp=>self.page_rows(tx,false),KeyCode::Char(']')|KeyCode::PageDown=>self.page_rows(tx,true),
                KeyCode::Up=>self.row_move(tx,-1),KeyCode::Down=>self.row_move(tx,1),
                KeyCode::Left=>self.horizontal=self.horizontal.saturating_sub(1),KeyCode::Right=>self.horizontal=self.horizontal.saturating_add(1),
                KeyCode::Enter=>{
                    let rows=sorted_rows(&self.panel_data(&panel),&panel);
                    if let Some(row)=rows.get(self.row.min(rows.len().saturating_sub(1))) {self.history(tx,"history.context",Some(row.clone()));}
                },
                KeyCode::Char('+')|KeyCode::Char('=')|KeyCode::Char('-')=>self.set(tx,true,json!({"view_zoom":(number(&page,"view_zoom",1.)*if key.code==KeyCode::Char('-'){0.8}else{1.25}).clamp(0.2,2.)})),
                KeyCode::Char('0')=>self.fit(tx),_=>{}
            }
        } else if let Event::Mouse(mouse) = event {
            if self.modal.is_some() {
                return Ok(true);
            }
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Middle) => {
                    self.begin_pan(mouse.column, mouse.row)
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if self.page().is_some_and(|p| p["tool"] == "pan")
                        && self.body.contains((mouse.column, mouse.row).into())
                    {
                        self.begin_pan(mouse.column, mouse.row);
                        return Ok(true);
                    }
                    if let Some(hit) = self
                        .hits
                        .iter()
                        .rev()
                        .find(|h| h.area.contains((mouse.column, mouse.row).into()))
                        .cloned()
                    {
                        let resize_hit = matches!(hit.kind, HitKind::Resize(_));
                        match hit.kind {
                            HitKind::Page(id) => self.send(
                                tx,
                                Call::new(
                                    "page.select",
                                    json!({"page":id,"revision":self.revision()}),
                                ),
                            ),
                            HitKind::Source(index) => self.add_stream(tx, index),
                            HitKind::Panel(id) => self.set(tx, true, json!({"active_panel":id})),
                            HitKind::Header(id) | HitKind::Resize(id) => {
                                if let Some(p) =
                                    self.panels().iter().find(|p| p["id"] == id).cloned()
                                {
                                    let resize = resize_hit;
                                    self.begin_draft(p, (mouse.column, mouse.row), resize, true);
                                }
                            }
                            HitKind::Row(id, row) => {
                                self.row = row;
                                self.set(tx, true, json!({"active_panel":id}));
                            }
                        }
                    }
                }
                MouseEventKind::Drag(MouseButton::Left | MouseButton::Middle) => {
                    if let Some(p) = &mut self.pan {
                        p.current = (
                            p.origin.0 + (f64::from(mouse.column) - f64::from(p.start.0)) * 8.,
                            p.origin.1 + (f64::from(mouse.row) - f64::from(p.start.1)) * 16.,
                        );
                        return Ok(true);
                    }
                    let zoom = self.page().map_or(1., |p| number(p, "view_zoom", 1.));
                    if let Some(d) = self.draft.as_mut() {
                        let (dx, dy) = (
                            (f64::from(mouse.column) - f64::from(d.start.0)) * 8. / zoom,
                            (f64::from(mouse.row) - f64::from(d.start.1)) * 16. / zoom,
                        );
                        d.panel = d.original.clone();
                        self.adjust_draft(dx, dy);
                    }
                }
                MouseEventKind::Up(MouseButton::Left | MouseButton::Middle) => {
                    if let Some(p) = self.pan.take() {
                        self.send(tx,Call::new("page.set",json!({"page":self.page_id(),"view_x":p.current.0,"view_y":p.current.1,"revision":p.revision})));
                    } else {
                        self.commit_draft(tx);
                    }
                }
                MouseEventKind::ScrollUp => self.row_move(tx, -3),
                MouseEventKind::ScrollDown => self.row_move(tx, 3),
                _ => {}
            }
        } else if let Event::Paste(value) = event {
            if let Some(Modal::Prompt(p)) = &mut self.modal {
                for c in value.chars().filter(|c| !c.is_control()).take(16384) {
                    insert(p, c);
                }
            }
        }
        Ok(true)
    }
    fn begin_pan(&mut self, x: u16, y: u16) {
        let page = self.page().cloned().unwrap_or_default();
        let origin = (number(&page, "view_x", 0.), number(&page, "view_y", 0.));
        self.pan = Some(Pan {
            start: (x, y),
            origin,
            current: origin,
            revision: self.revision(),
        });
    }
    fn next_panel(&mut self, tx: &mpsc::Sender<Call>, back: bool) {
        let panels: Vec<_> = self
            .panels()
            .into_iter()
            .filter(|p| p["hidden"] != true)
            .collect();
        if panels.is_empty() {
            return;
        }
        let current = self.panel_id();
        let n = panels
            .iter()
            .position(|p| p["id"].as_str() == current.as_deref())
            .unwrap_or(0);
        let index = if back {
            (n + panels.len() - 1) % panels.len()
        } else {
            (n + 1) % panels.len()
        };
        self.set(tx, true, json!({"active_panel":panels[index]["id"]}));
    }
    fn row_move(&mut self, tx: &mpsc::Sender<Call>, delta: isize) {
        if let Some(p) = self.selected().cloned() {
            let n = sorted_rows(&self.panel_data(&p), &p).len();
            if p["follow"] == true {
                self.row = n.saturating_sub(1);
                self.set(tx, false, json!({"follow":false}));
            }
            self.row = self
                .row
                .saturating_add_signed(delta)
                .min(n.saturating_sub(1));
        }
    }
    fn page_rows(&mut self, tx: &mpsc::Sender<Call>, next: bool) {
        if let Some(p) = self.selected().cloned() {
            let offset = p["offset"].as_u64().unwrap_or(0);
            let data = self.panel_data(&p);
            let value = if next {
                data["next"].as_u64().unwrap_or(offset + 200)
            } else {
                offset.saturating_sub(200)
            };
            if p["mode"] == "history" && (!next || value < data["total"].as_u64().unwrap_or(0)) {
                self.row = 0;
                self.scroll = 0;
                self.set(tx, false, json!({"offset":value}));
            }
        }
    }
    fn history(&mut self, tx: &mpsc::Sender<Call>, method: &str, row: Option<Value>) {
        if let Some(p) = self.selected().cloned() {
            let page = self.page_id();
            let panel = text(&p, "id");
            let mut args = json!({"page":page,"panel":panel,"streams":p["streams"],"channels":p["channels"],"text":p["text"],"regex":p["regex"],"time_from":p["time_from"],"time_end":p["time_end"],"revision":self.revision()});
            if let Some(r) = row {
                args["streams"] = json!([r["stream"]]);
                for k in ["epoch", "seq"] {
                    args[k] = r[k].clone();
                }
                args["byte_offset"] = r["offset"].clone();
                args["before"] = json!(10);
                args["after"] = json!(10);
                args["text"] = json!("");
                args["regex"] = json!("");
            }
            let method = if p["kind"] == "curve" {
                "history.curve"
            } else {
                method
            };
            self.send(
                tx,
                Call {
                    method: method.into(),
                    args,
                    after: Some(After::ShowQuery { page, panel }),
                },
            );
        }
    }
    fn add_stream(&mut self, tx: &mpsc::Sender<Call>, index: usize) {
        if let Some(s) = self.snapshot.state["streams"]
            .as_array()
            .and_then(|s| s.get(index))
            .cloned()
        {
            self.add(tx, "log", Some(s));
        }
    }
    fn add(&mut self, tx: &mpsc::Sender<Call>, kind: &str, source: Option<Value>) {
        let n = self.panels().len();
        let mut args = json!({"page":self.page_id(),"kind":kind,"title":if kind=="log"{"Logs"}else{"Curve"},"left":24+(n%5)*32,"top":16+(n%5)*32,"panel_width":640,"panel_height":320,"x":0,"y":n*4,"w":12,"h":4,"z_index":n,"revision":self.revision()});
        if let Some(s) = source {
            args["title"] = s["alias"]
                .as_str()
                .map(|v| json!(v))
                .unwrap_or_else(|| s["owner"].clone());
            args["streams"] = json!([if s["alias"].is_string() {
                json!({"owner":s["owner"],"alias":s["alias"]})
            } else {
                json!({"owner":s["owner"],"alias":null,"stream":s["id"],"epoch":s["epoch"]})
            }]);
        }
        self.send(
            tx,
            Call {
                method: "panel.add".into(),
                args,
                after: Some(After::SelectPanel),
            },
        );
    }
    fn begin_draft(&mut self, panel: Value, start: (u16, u16), resize: bool, mouse: bool) {
        if panel["locked"] == true || self.page().is_some_and(|p| p["locked"] == true) {
            self.status = "Layout is locked; use L or page inspector to unlock".into();
            return;
        }
        self.status = "Layout draft · arrows adjust · Enter saves · Esc discards".into();
        self.draft = Some(Draft {
            original: panel.clone(),
            panel,
            revision: self.revision(),
            start,
            resize,
            mouse,
        });
    }
    fn adjust_draft(&mut self, dx: f64, dy: f64) {
        let grid = self.page().is_some_and(|p| p["layout_mode"] == "grid");
        let Some(d) = &mut self.draft else {
            return;
        };
        if grid {
            let (a, b) = if d.resize { ("w", "h") } else { ("x", "y") };
            let min = if d.resize { 1. } else { 0. };
            d.panel[a] = json!((number(&d.panel, a, 1.) + dx / 8.).round().clamp(min, 12.));
            d.panel[b] = json!((number(&d.panel, b, 1.) + dy / 16.)
                .round()
                .clamp(min, 100.));
            let x = number(&d.panel, "x", 0.).min(11.);
            d.panel["x"] = json!(x as u64);
            d.panel["w"] = json!(number(&d.panel, "w", 1.).min(12. - x) as u64);
            d.panel["y"] = json!(number(&d.panel, "y", 0.) as u64);
            d.panel["h"] = json!(number(&d.panel, "h", 1.) as u64);
        } else {
            let (a, b, min_a, min_b, max) = if d.resize {
                ("panel_width", "panel_height", 320., 220., 4000.)
            } else {
                ("left", "top", -1_000_000., -1_000_000., 1_000_000.)
            };
            d.panel[a] = json!((number(&d.panel, a, 0.) + dx).clamp(min_a, max));
            d.panel[b] = json!((number(&d.panel, b, 0.) + dy).clamp(min_b, max));
        }
    }
    fn commit_draft(&mut self, tx: &mpsc::Sender<Call>) {
        if let Some(d) = self.draft.take() {
            let mut p = d.panel;
            if self
                .page()
                .is_some_and(|p| p["snap"] == true && p["layout_mode"] != "grid")
            {
                for key in ["left", "top"] {
                    p[key] = json!((number(&p, key, 0.) / 8.).round() * 8.);
                }
            }
            self.send(tx,Call::new("layout.set",json!({"page":self.page_id(),"revision":d.revision,"active_panel":p["id"],"layout":[{"id":p["id"],"left":p["left"],"top":p["top"],"panel_width":p["panel_width"],"panel_height":p["panel_height"],"x":p["x"],"y":p["y"],"w":p["w"],"h":p["h"]}]})));
        }
    }
    fn fit(&mut self, tx: &mpsc::Sender<Call>) {
        let ps: Vec<_> = self
            .panels()
            .into_iter()
            .filter(|p| p["hidden"] != true)
            .collect();
        if ps.is_empty() {
            return;
        }
        let left = ps
            .iter()
            .map(|p| number(p, "left", 0.))
            .fold(f64::INFINITY, f64::min);
        let top = ps
            .iter()
            .map(|p| number(p, "top", 0.))
            .fold(f64::INFINITY, f64::min);
        let right = ps
            .iter()
            .map(|p| number(p, "left", 0.) + number(p, "panel_width", 640.))
            .fold(f64::NEG_INFINITY, f64::max);
        let bottom = ps
            .iter()
            .map(|p| number(p, "top", 0.) + number(p, "panel_height", 320.))
            .fold(f64::NEG_INFINITY, f64::max);
        let z = ((f64::from(self.body.width.saturating_sub(2)) * 8.) / (right - left))
            .min((f64::from(self.body.height.saturating_sub(2)) * 16.) / (bottom - top))
            .clamp(0.2, 2.);
        self.set(
            tx,
            true,
            json!({"view_x":8.-left*z,"view_y":16.-top*z,"view_zoom":z}),
        );
    }
    fn modal_key(&mut self, key: KeyEvent, tx: &mpsc::Sender<Call>) {
        let Some(mut modal) = self.modal.take() else {
            return;
        };
        if key.code == KeyCode::Esc {
            return;
        }
        match &mut modal {
            Modal::Prompt(p) => match key.code {
                KeyCode::Enter => {
                    if let Err(e) = self.submit(p.clone(), tx) {
                        self.status = e.to_string();
                        self.modal = Some(modal);
                    }
                    return;
                }
                KeyCode::Left => p.cursor = p.cursor.saturating_sub(1),
                KeyCode::Right => p.cursor = (p.cursor + 1).min(p.text.chars().count()),
                KeyCode::Home => p.cursor = 0,
                KeyCode::End => p.cursor = p.text.chars().count(),
                KeyCode::Backspace => {
                    if p.cursor > 0 {
                        p.cursor -= 1;
                        erase(p, p.cursor);
                    }
                }
                KeyCode::Delete => erase(p, p.cursor),
                KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => p.cursor = 0,
                KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    p.cursor = p.text.chars().count()
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => insert(p, c),
                _ => {}
            },
            Modal::Menu { kind, index } => {
                let n = self.menu_items(*kind).len();
                match key.code {
                    KeyCode::Up => *index = index.saturating_sub(1),
                    KeyCode::Down => *index = (*index + 1).min(n.saturating_sub(1)),
                    KeyCode::Enter
                    | KeyCode::Char('a' | 'n' | 'c' | 'd' | 'i' | 'e' | ' ')
                    | KeyCode::Delete => {
                        self.menu_action(*kind, *index, key.code, tx);
                        return;
                    }
                    _ => {}
                }
            }
            Modal::Info { offset, .. } => match key.code {
                KeyCode::Up => *offset = offset.saturating_sub(1),
                KeyCode::Down => *offset = offset.saturating_add(1),
                KeyCode::PageDown => *offset = offset.saturating_add(10),
                KeyCode::PageUp => *offset = offset.saturating_sub(10),
                _ => {}
            },
            Modal::Confirm { method, args, .. } => {
                if key.code == KeyCode::Enter {
                    self.send(tx, Call::new(method, args.clone()));
                    return;
                }
            }
            Modal::Result => {}
        }
        self.modal = Some(modal);
    }
    fn submit(&mut self, p: Prompt, tx: &mpsc::Sender<Call>) -> Result<()> {
        let mut call = match p.kind {
            PromptKind::Command => {
                let words = shell_words::split(&p.text)?;
                let cli = Palette::try_parse_from(std::iter::once(String::new()).chain(words))?;
                let (method, mut args) = cli.command.request()?;
                if ((method.starts_with("page.")
                    && !matches!(method.as_str(), "page.create" | "page.list"))
                    || method.starts_with("panel.")
                    || method.starts_with("layout.")
                    || method.starts_with("series.")
                    || method.starts_with("history."))
                    && args.get("page").is_none()
                {
                    args["page"] = json!(p.page);
                }
                if ((method.starts_with("panel.") && method != "panel.add")
                    || method.starts_with("series.")
                    || method == "history.curve")
                    && args.get("panel").is_none()
                {
                    args["panel"] = json!(p.panel);
                }
                let after = match method.as_str() {
                    "page.create" | "page.clone" => Some(After::SelectPage),
                    "panel.add" | "panel.clone" => Some(After::SelectPanel),
                    m if m.starts_with("history.") && p.panel.is_some() => Some(After::ShowQuery {
                        page: args["page"].as_str().unwrap_or(&p.page).to_owned(),
                        panel: args["panel"]
                            .as_str()
                            .or(p.panel.as_deref())
                            .unwrap()
                            .to_owned(),
                    }),
                    _ => None,
                };
                if after.is_none()
                    && (method.ends_with(".get")
                        || method.ends_with(".list")
                        || ["url", "streams", "capabilities"].contains(&method.as_str()))
                {
                    self.modal = Some(Modal::Result);
                }
                Call {
                    method,
                    args,
                    after,
                }
            }
            PromptKind::PageName => {
                if p.text.trim().is_empty() {
                    bail!("Page name is required");
                }
                Call {
                    method: "page.create".into(),
                    args: json!({"name":p.text,"title":p.text}),
                    after: Some(After::SelectPage),
                }
            }
            PromptKind::Filter | PromptKind::Regex => Call::new(
                "panel.set",
                json!({"page":p.page,"panel":p.panel,if matches!(p.kind,PromptKind::Regex){"regex"}else{"text"}:p.text}),
            ),
            PromptKind::Field { page, key, kind } => {
                let value = match kind {
                    FieldType::Text => json!(p.text),
                    FieldType::Bool => {
                        json!(p.text.parse::<bool>().context("Use true or false")?)
                    }
                    FieldType::Number => {
                        if p.text.is_empty() {
                            Value::Null
                        } else {
                            serde_json::from_str::<Value>(&p.text).context("Enter a number")?
                        }
                    }
                    FieldType::List => json!(p
                        .text
                        .split(',')
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .collect::<Vec<_>>()),
                };
                let mut args = json!({"page":p.page,key:value});
                if !page {
                    args["panel"] = json!(p.panel);
                }
                Call::new(if page { "page.set" } else { "panel.set" }, args)
            }
        };
        if call.args.get("revision").is_none() {
            call.args["revision"] = json!(p.revision);
        }
        self.send(tx, call);
        Ok(())
    }
    pub fn menu_items(&self, kind: Menu) -> Vec<String> {
        match kind {
            Menu::Pages => self.snapshot.state["pages"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    format!(
                        "{}  ·  {} panels",
                        text(p, "name"),
                        p["panels"].as_array().map_or(0, Vec::len)
                    )
                })
                .collect(),
            Menu::Streams => self.snapshot.state["streams"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| {
                    format!(
                        "{} / {}  ·  {}",
                        text(s, "owner"),
                        text(s, "alias"),
                        s["writer"].as_str().unwrap_or("stream")
                    )
                })
                .collect(),
            Menu::Series => self
                .selected()
                .and_then(|p| p["series"].as_array())
                .into_iter()
                .flatten()
                .map(|s| {
                    format!(
                        "{}  {}  {}",
                        text(s, "name"),
                        text(s, "color"),
                        if s["field"].is_string() {
                            text(s, "field")
                        } else {
                            text(s, "regex")
                        }
                    )
                })
                .collect(),
            Menu::Panel | Menu::Page => {
                let page = kind == Menu::Page;
                let v = if page { self.page() } else { self.selected() };
                fields(page)
                    .iter()
                    .map(|(label, key, _)| {
                        format!(
                            "{label:<18} {}",
                            v.map(|v| display(&v[*key])).unwrap_or_default()
                        )
                    })
                    .collect()
            }
        }
    }
    fn menu_action(&mut self, kind: Menu, index: usize, key: KeyCode, tx: &mpsc::Sender<Call>) {
        match kind {
            Menu::Pages => {
                let p = self.snapshot.state["pages"]
                    .as_array()
                    .and_then(|a| a.get(index))
                    .cloned()
                    .unwrap_or(Value::Null);
                match key {
                    KeyCode::Enter => self.send(
                        tx,
                        Call::new(
                            "page.select",
                            json!({"page":p["id"],"revision":self.revision()}),
                        ),
                    ),
                    KeyCode::Char('n' | 'a') => {
                        self.prompt("Create Page", String::new(), PromptKind::PageName)
                    }
                    KeyCode::Char('c') => self.prompt(
                        "Clone Page",
                        format!(
                            "page clone --page {} --name {}-copy",
                            text(&p, "id"),
                            shell_words::quote(&text(&p, "name"))
                        ),
                        PromptKind::Command,
                    ),
                    KeyCode::Delete | KeyCode::Char('d') => {
                        self.modal = Some(Modal::Confirm {
                            method: "page.delete".into(),
                            args: json!({"page":p["id"],"revision":self.revision()}),
                            title: format!("Delete Page {}?", text(&p, "name")),
                        })
                    }
                    _ => {}
                }
            }
            Menu::Streams => {
                if key == KeyCode::Enter {
                    self.add_stream(tx, index);
                } else if let Some(s) = self.snapshot.state["streams"]
                    .as_array()
                    .and_then(|a| a.get(index))
                {
                    self.modal = Some(Modal::Info {
                        title: "Stream identity / coverage".into(),
                        text: serde_json::to_string_pretty(s).unwrap_or_default(),
                        offset: 0,
                    });
                }
            }
            Menu::Series => {
                let panel = self.selected().cloned().unwrap_or(Value::Null);
                let series = panel["series"]
                    .as_array()
                    .and_then(|a| a.get(index))
                    .cloned()
                    .unwrap_or(Value::Null);
                match key {
                    KeyCode::Char('a' | 'n') => self.prompt(
                        "Add series",
                        "series add --name Value --field value --color '#5aa9fa'".into(),
                        PromptKind::Command,
                    ),
                    KeyCode::Enter | KeyCode::Char('e') => self.prompt(
                        "Edit series",
                        format!(
                            "series set --series {} --name {} --color {} --width {} {} {}",
                            text(&series, "id"),
                            shell_words::quote(&text(&series, "name")),
                            shell_words::quote(&text(&series, "color")),
                            series["width"].as_f64().unwrap_or(2.),
                            if series["field"].as_str().is_some_and(|s| !s.is_empty()) {
                                "--field"
                            } else {
                                "--regex"
                            },
                            shell_words::quote(&if series["field"]
                                .as_str()
                                .is_some_and(|s| !s.is_empty())
                            {
                                text(&series, "field")
                            } else {
                                text(&series, "regex")
                            })
                        ),
                        PromptKind::Command,
                    ),
                    KeyCode::Delete | KeyCode::Char('d') => {
                        self.modal = Some(Modal::Confirm {
                            method: "series.remove".into(),
                            args: json!({"page":self.page_id(),"panel":panel["id"],"series":series["id"],"revision":self.revision()}),
                            title: format!("Remove series {}?", text(&series, "name")),
                        })
                    }
                    KeyCode::Char(' ') => {
                        let mut legend = panel["legend_selected"]
                            .as_object()
                            .cloned()
                            .unwrap_or_default();
                        let name = text(&series, "name");
                        let on = legend.get(&name).is_some_and(|v| v == false);
                        legend.insert(name, json!(on));
                        self.set(tx, false, json!({"legend_selected":legend}));
                    }
                    _ => {}
                }
            }
            Menu::Page | Menu::Panel => {
                let page = kind == Menu::Page;
                if let Some((label, k, field_type)) = fields(page).get(index) {
                    let v = if page { self.page() } else { self.selected() }
                        .cloned()
                        .unwrap_or(Value::Null);
                    if matches!(field_type, FieldType::Bool) {
                        self.set(tx, page, json!({*k:v[*k]!=true}));
                    } else {
                        let value = if matches!(field_type, FieldType::List) {
                            v[*k]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|x| x.as_str().unwrap_or_default())
                                .collect::<Vec<_>>()
                                .join(",")
                        } else if v[*k].is_null() {
                            String::new()
                        } else {
                            display(&v[*k])
                        };
                        self.prompt(
                            label,
                            value,
                            PromptKind::Field {
                                page,
                                key: (*k).into(),
                                kind: *field_type,
                            },
                        );
                    }
                }
            }
        }
    }
}
pub fn number(v: &Value, key: &str, default: f64) -> f64 {
    v[key].as_f64().unwrap_or(default)
}
pub fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}
pub fn display(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}
// Untrusted logs and names must never become terminal control sequences.
pub fn safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() {
                format!("\\u{:04x}", c as u32)
            } else {
                c.to_string()
            }
        })
        .collect()
}
fn erase(p: &mut Prompt, index: usize) {
    if let Some((byte, c)) = p.text.char_indices().nth(index) {
        p.text.replace_range(byte..byte + c.len_utf8(), "");
    }
}
fn insert(p: &mut Prompt, c: char) {
    if c.is_control() || p.text.len() >= 16384 {
        return;
    }
    let byte = p
        .text
        .char_indices()
        .nth(p.cursor)
        .map_or(p.text.len(), |v| v.0);
    p.text.insert(byte, c);
    p.cursor += 1;
}
pub fn sorted_rows(data: &Value, panel: &Value) -> Vec<Value> {
    let mut rows = data["rows"].as_array().cloned().unwrap_or_default();
    let mut sorts: Vec<_> = panel["column_state"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v["sort"].is_string())
        .collect();
    sorts.sort_by_key(|v| v["sortIndex"].as_u64().unwrap_or(0));
    if !sorts.is_empty() {
        rows.sort_by(|a, b| {
            for s in &sorts {
                let key = s["colId"].as_str().unwrap_or_default();
                let cmp = if let (Some(a), Some(b)) = (a[key].as_f64(), b[key].as_f64()) {
                    a.total_cmp(&b)
                } else {
                    display(&a[key]).cmp(&display(&b[key]))
                };
                let cmp = if s["sort"] == "desc" {
                    cmp.reverse()
                } else {
                    cmp
                };
                if !cmp.is_eq() {
                    return cmp;
                }
            }
            std::cmp::Ordering::Equal
        });
    }
    rows
}
pub fn fields(page: bool) -> &'static [(&'static str, &'static str, FieldType)] {
    use FieldType::*;
    if page {
        &[
            ("Name", "name", Text),
            ("Title", "title", Text),
            ("Theme", "theme", Text),
            ("Order", "order", Number),
            ("Layout", "layout_mode", Text),
            ("Pan X", "view_x", Number),
            ("Pan Y", "view_y", Number),
            ("Zoom", "view_zoom", Number),
            ("Grid", "show_grid", Bool),
            ("Snap", "snap", Bool),
            ("Locked", "locked", Bool),
            ("Minimap", "show_minimap", Bool),
            ("Sidebar", "sidebar_open", Bool),
            ("Inspector", "inspector_open", Bool),
            ("Tool", "tool", Text),
        ]
    } else {
        &[
            ("Title", "title", Text),
            ("Streams (comma)", "streams", List),
            ("Channels (comma)", "channels", List),
            ("Text filter", "text", Text),
            ("Regex filter", "regex", Text),
            ("Format", "format", Text),
            ("Columns (comma)", "columns", List),
            ("Metadata", "metadata", Bool),
            ("Follow", "follow", Bool),
            ("Paused", "paused", Bool),
            ("Mode", "mode", Text),
            ("Left", "left", Number),
            ("Top", "top", Number),
            ("Width px", "panel_width", Number),
            ("Height px", "panel_height", Number),
            ("Z order", "z_index", Number),
            ("Hidden", "hidden", Bool),
            ("Locked", "locked", Bool),
            ("Grid X", "x", Number),
            ("Grid Y", "y", Number),
            ("Grid W", "w", Number),
            ("Grid H", "h", Number),
            ("Time from ns", "time_from", Number),
            ("Time end ns", "time_end", Number),
            ("Y min", "y_min", Number),
            ("Y max", "y_max", Number),
            ("Legend", "legend", Bool),
            ("Zoom start %", "zoom_start", Number),
            ("Zoom end %", "zoom_end", Number),
            ("Row height px", "row_height", Number),
            ("Font size px", "font_size", Number),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_uses_current_selection_and_captured_revision_without_shell_execution() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::channel(4);
        let p = Prompt {
            title: String::new(),
            text: "panel set --title '$(touch NEVER_EXECUTE)' --left -24".into(),
            cursor: 0,
            kind: PromptKind::Command,
            revision: 7,
            page: "page".into(),
            panel: Some("panel".into()),
        };
        app.submit(p, &tx).unwrap();
        let call = rx.try_recv().unwrap();
        assert_eq!(call.method, "panel.set");
        assert_eq!(
            call.args,
            json!({"title":"$(touch NEVER_EXECUTE)","left":-24.,"page":"page","panel":"panel","revision":7})
        );
    }
    #[test]
    fn utf8_editing_and_control_characters_do_not_corrupt_prompt() {
        let mut p = Prompt {
            title: String::new(),
            text: "温度".into(),
            cursor: 1,
            kind: PromptKind::Filter,
            revision: 0,
            page: String::new(),
            panel: None,
        };
        insert(&mut p, '🌡');
        insert(&mut p, '\u{1b}');
        assert_eq!(p.text, "温🌡度");
        erase(&mut p, 1);
        assert_eq!(p.text, "温度");
        assert_eq!(safe("a\u{1b}[2J\t"), "a\\u001b[2J\\u0009");
    }
}
