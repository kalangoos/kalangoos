//! The files of the Desktop folder on the desktop (design "Área de trabalho · arquivos"): each file and folder of
//! `xdg-user-dir DESKTOP` as an icon — Files' folder art, the system's thumbnails, an app's own icon for a launcher —
//! with its name on up to two lines, on a grid that keeps clear of the widgets.
//!
//! - **Where**: a grid of 104×112 px cells counted from the right edge of the screen. Auto-arranged icons start in the
//!   first column left of the right-hand widgets and go down, column by column, to the left; cells under a widget
//!   (16 px around it) are skipped; when the middle is full the columns under the right-hand widgets are used.
//!   With no widget on the right (a short screen keeps only the clock) the icons start at the right edge.
//!   What does not fit becomes the last cell's "+N" (opens the folder in Files).
//! - **Order**: folders first, then by name (numbers in names in their order); "Organizar ícones" puts every icon
//!   back in that order.
//! - **Position memory**: an icon dragged to a cell stays there (by its name, its cell counted from the right edge, in
//!   `~/.config/kalango/desktop-icons.json`), also after a rename; a cell taken by a widget sends it back to the order.
//! - **Live**: a `gio::FileMonitor` on the folder: whatever app creates, renames or deletes a file there, the desktop
//!   follows (the icon comes in and goes out with the gentle spring; one that moves slides there).
//! - **Using them**: a click selects (Ctrl adds, Shift a range, dragging on the empty desktop a rubber band), a double
//!   click or Enter opens (with the default app; a folder in Files; a launcher runs when it is executable), F2 renames in
//!   place, Delete sends to the trash with Undo (the notice's button or Ctrl+Z), Ctrl+C/X/V copy and move, Ctrl+A, the
//!   arrows move between icons. The menu: Abrir, Abrir com, Copiar, Recortar, Renomear, Mostrar no Arquivos, Pedir ao
//!   agente, Mover para a lixeira. Icons drag to Files, to apps and to a folder icon; files from Files dropped on the
//!   desktop move into the folder, where they were dropped.
//! - Settings › Aparência › "Arquivos na área de trabalho" shows or hides them.

use crate::arquivos::{self, Alvo, Item};
use crate::superficies::{caixa, rotulo, BARRA_ALTURA};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use serde::{Deserialize, Serialize};
use std::cell::{Cell as StdCell, RefCell};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

// ---------------------------------------------------------------- the grid (pure)

/// An icon's tile, and the grid's step (8 px between tiles).
pub const TILE_W: i32 = 96;
pub const TILE_H: i32 = 104;
pub const STEP_X: i32 = 104;
pub const STEP_Y: i32 = 112;
/// From the edges of the desktop (under the top bar).
pub const EDGE: i32 = 24;
/// The room kept around a widget.
pub const CLEAR: i32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    fn grown(&self, d: i32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2 * d, self.h + 2 * d)
    }

    /// From two corners, in any order (a rubber band).
    pub fn from_corners(a: (f64, f64), b: (f64, f64)) -> Rect {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        Rect::new(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32)
    }
}

/// A cell: its column counted from the right edge (0 is the rightmost), its row from the top.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Cell {
    pub col: i32,
    pub row: i32,
}

impl Cell {
    pub fn new(col: i32, row: i32) -> Cell {
        Cell { col, row }
    }
}

/// The desktop's size (the surface under the top bar) and what icons must keep clear of (the widgets on it).
pub struct Grid {
    pub width: i32,
    pub height: i32,
    pub obstacles: Vec<Rect>,
}

impl Grid {
    pub fn cols(&self) -> i32 {
        if self.width < 2 * EDGE + TILE_W { 0 } else { (self.width - 2 * EDGE - TILE_W) / STEP_X + 1 }
    }

    pub fn rows(&self) -> i32 {
        if self.height < 2 * EDGE + TILE_H { 0 } else { (self.height - 2 * EDGE - TILE_H) / STEP_Y + 1 }
    }

    pub fn rect(&self, c: Cell) -> Rect {
        Rect::new(self.width - EDGE - TILE_W - c.col * STEP_X, EDGE + c.row * STEP_Y, TILE_W, TILE_H)
    }

    pub fn inside(&self, c: Cell) -> bool {
        c.col >= 0 && c.row >= 0 && c.col < self.cols() && c.row < self.rows()
    }

    /// On the screen and clear of every widget.
    pub fn free(&self, c: Cell) -> bool {
        let r = self.rect(c);
        self.inside(c) && self.obstacles.iter().all(|o| !o.grown(CLEAR).overlaps(&r))
    }

    /// The first column left of the widgets on the right half of the screen (0 when there are none).
    fn first_col(&self) -> i32 {
        let right: Vec<Rect> = self.obstacles.iter().filter(|o| o.x + o.w / 2 > self.width / 2).map(|o| o.grown(CLEAR)).collect();
        (0..self.cols())
            .find(|&k| {
                let r = self.rect(Cell::new(k, 0));
                right.iter().all(|o| r.x >= o.x + o.w || r.x + r.w <= o.x)
            })
            .unwrap_or(0)
    }

    /// The free cells in the order auto-arranged icons take them: down each column, from the first column left of
    /// the right-hand widgets to the left edge, then the columns under those widgets.
    pub fn order(&self) -> Vec<Cell> {
        let k0 = self.first_col();
        let cols: Vec<i32> = (k0..self.cols()).chain(0..k0).collect();
        let mut v = Vec::new();
        for k in cols {
            for r in 0..self.rows() {
                let c = Cell::new(k, r);
                if self.free(c) {
                    v.push(c);
                }
            }
        }
        v
    }

    /// The cell nearest to a tile whose top left corner is at (x, y).
    pub fn snap(&self, x: i32, y: i32) -> Cell {
        let col = ((self.width - EDGE - TILE_W - x) as f64 / STEP_X as f64).round() as i32;
        let row = ((y - EDGE) as f64 / STEP_Y as f64).round() as i32;
        Cell::new(col.clamp(0, (self.cols() - 1).max(0)), row.clamp(0, (self.rows() - 1).max(0)))
    }

    /// The free cell not in `taken` nearest to `want` (itself when it is free).
    pub fn nearest_free(&self, want: Cell, taken: &HashSet<Cell>) -> Option<Cell> {
        let ok = |c: Cell| self.free(c) && !taken.contains(&c);
        if ok(want) {
            return Some(want);
        }
        let d = |c: &Cell| {
            let (a, b) = (self.rect(*c), self.rect(want));
            let (dx, dy) = (i64::from(a.x - b.x), i64::from(a.y - b.y));
            dx * dx + dy * dy
        };
        self.order().into_iter().filter(|c| !taken.contains(c)).min_by_key(d)
    }
}

/// Where each icon goes: (index in the order given, cell), and the "+N" cell with how many did not fit.
#[derive(Debug, PartialEq, Eq)]
pub struct Placement {
    pub cells: Vec<(usize, Cell)>,
    pub more: Option<(Cell, usize)>,
}

/// Places icons (`names` in their order): a remembered cell first, when it is free and not taken by an earlier one;
/// the others in the grid's order. When they do not all fit, the last free cell is "+N".
pub fn place(grid: &Grid, names: &[String], memory: &BTreeMap<String, Cell>) -> Placement {
    let mut taken: HashSet<Cell> = HashSet::new();
    let mut cells: Vec<(usize, Cell)> = Vec::new();
    let mut rest = Vec::new();
    for (i, n) in names.iter().enumerate() {
        match memory.get(n).copied().filter(|c| grid.free(*c) && !taken.contains(c)) {
            Some(c) => {
                taken.insert(c);
                cells.push((i, c));
            }
            None => rest.push(i),
        }
    }
    let free: Vec<Cell> = grid.order().into_iter().filter(|c| !taken.contains(c)).collect();
    let mut more = None;
    if rest.len() > free.len() {
        if free.is_empty() {
            // (not even a cell for "+N": the remembered ones only)
            return Placement { cells, more: None };
        }
        let fit = free.len() - 1;
        more = Some((free[fit], rest.len() - fit));
        rest.truncate(fit);
    }
    cells.extend(rest.into_iter().zip(free));
    cells.sort_by_key(|(i, _)| *i);
    Placement { cells, more }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The icon an arrow key goes to from `from`: the nearest one that way (on the same row or column first).
pub fn neighbour(cells: &[Cell], from: Cell, dir: Dir) -> Option<usize> {
    cells
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            // (columns count from the right: Left is a larger column)
            let (ahead, side) = match dir {
                Dir::Left => (c.col - from.col, c.row - from.row),
                Dir::Right => (from.col - c.col, c.row - from.row),
                Dir::Up => (from.row - c.row, c.col - from.col),
                Dir::Down => (c.row - from.row, c.col - from.col),
            };
            (ahead > 0).then_some((i, (side.abs(), ahead)))
        })
        .min_by_key(|(_, k)| *k)
        .map(|(i, _)| i)
}

/// Names compared as people read them: case aside, and "Foto 2" before "Foto 10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    fn parts(s: &str) -> Vec<(bool, String)> {
        let mut v: Vec<(bool, String)> = Vec::new();
        for ch in s.chars() {
            let d = ch.is_ascii_digit();
            match v.last_mut() {
                Some((ld, t)) if *ld == d => t.push(ch),
                _ => v.push((d, ch.to_string())),
            }
        }
        v
    }
    let (pa, pb) = (parts(&a.to_lowercase()), parts(&b.to_lowercase()));
    for ((da, ta), (db, tb)) in pa.iter().zip(pb.iter()) {
        let o = if *da && *db {
            let (na, nb) = (ta.trim_start_matches('0'), tb.trim_start_matches('0'));
            na.len().cmp(&nb.len()).then_with(|| na.cmp(nb))
        } else {
            ta.cmp(tb)
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    pa.len().cmp(&pb.len()).then_with(|| a.cmp(b))
}

/// The desktop's order: folders first, then by name.
pub fn sort_entries(v: &mut [(String, bool)]) {
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| natural_cmp(&a.0, &b.0)));
}

/// Where the person put each icon (by its name).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    #[serde(default)]
    pub positions: BTreeMap<String, Cell>,
}

/// Never more remembered places than this (the oldest names go first… alphabetically: they are not dated).
const MEMORY_MAX: usize = 1000;

impl Memory {
    fn path() -> PathBuf {
        glib::user_config_dir().join("kalango/desktop-icons.json")
    }

    pub fn load() -> Memory {
        std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = Self::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }

    pub fn put(&mut self, name: &str, c: Cell) {
        // a cell holds one icon
        self.positions.retain(|n, v| n == name || *v != c);
        self.positions.insert(name.to_string(), c);
        while self.positions.len() > MEMORY_MAX {
            let first = self.positions.keys().next().cloned();
            if let Some(k) = first.filter(|k| k != name) {
                self.positions.remove(&k);
            } else {
                break;
            }
        }
    }

    /// A file renamed keeps its place.
    pub fn renamed(&mut self, old: &str, new: &str) {
        if let Some(c) = self.positions.remove(old) {
            self.positions.insert(new.to_string(), c);
        }
    }
}

// ---------------------------------------------------------------- the icons on the desktop

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// An entry of the folder: a launcher (`.desktop`) shows its app's name and icon.
fn launcher(i: &Item) -> Option<gio::DesktopAppInfo> {
    (!i.pasta && arquivos::extensao(&i.caminho) == "desktop").then(|| gio::DesktopAppInfo::from_filename(&i.caminho)).flatten()
}

fn executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o100 != 0)
}

/// A page drawn for a file with no thumbnail (design: Files' dark page of the icons view, at 44×56): four lines of
/// text; the kind's badge goes over it.
fn fake_page() -> gtk::Box {
    let p = caixa(Orientation::Vertical, 4, "arq-pagina-falsa");
    p.add_css_class("escura");
    p.set_size_request(44, 56);
    p.set_valign(Align::Start);
    for (k, w) in [25, 22, 27, 17].into_iter().enumerate() {
        let l = caixa(Orientation::Horizontal, 0, "arq-pagina-linha");
        l.set_size_request(w, 3);
        l.set_halign(Align::Start);
        l.set_margin_start(8);
        if k == 0 {
            l.set_margin_top(9);
        }
        p.append(&l);
    }
    p
}

struct Icon {
    item: RefCell<Item>,
    anim: Animavel,
    tile: gtk::Box,
    label: gtk::Label,
    art: gtk::Box,
    cell: StdCell<Option<Cell>>,
}

/// A drag that started on the desktop: the files, the one under the pointer and where the pointer was in its tile.
struct OwnDrag {
    primary: PathBuf,
    paths: Vec<PathBuf>,
    offset: (f64, f64),
}

pub struct DesktopFiles {
    area: Weak<crate::widgets::Area>,
    shell: Weak<Shell>,
    fixed: gtk::Fixed,
    window: gtk::ApplicationWindow,
    folder: PathBuf,
    icons: RefCell<Vec<Rc<Icon>>>,
    more: Animavel,
    more_label: gtk::Label,
    band: gtk::Box,
    band_start: RefCell<Option<(f64, f64, BTreeSet<PathBuf>)>>,
    selected: RefCell<BTreeSet<PathBuf>>,
    anchor: RefCell<Option<PathBuf>>,
    memory: RefCell<Memory>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    pending: StdCell<bool>,
    generation: StdCell<u64>,
    /// a new folder made here: renamed in place once it shows
    rename_next: RefCell<Option<PathBuf>>,
    renaming: RefCell<Option<(PathBuf, gtk::Entry)>>,
    drag: RefCell<Option<OwnDrag>>,
    textures: RefCell<HashMap<(PathBuf, i64, Alvo), gdk::Texture>>,
    /// what went to the trash from here last: (where it was, where it is in the trash)
    last_trashed: RefCell<Vec<(PathBuf, PathBuf)>>,
    enabled: StdCell<bool>,
    /// hidden while the widgets are edited
    suspended: StdCell<bool>,
}

impl DesktopFiles {
    pub fn new(area: &Rc<crate::widgets::Area>, shell: &Rc<Shell>) -> Rc<DesktopFiles> {
        let fixed = area.fixed();
        let more_label = rotulo("", "desk-icone-nome");
        let more_tile = caixa(Orientation::Vertical, 5, "desk-icone");
        more_tile.set_size_request(TILE_W, TILE_H);
        let more_art = caixa(Orientation::Vertical, 0, "desk-mais");
        more_art.set_size_request(56, 56);
        more_art.set_halign(Align::Center);
        more_art.set_margin_top(4);
        let mi = crate::conversa::ui_icon("mais", "#E2E2E6", 20);
        mi.set_vexpand(true);
        more_art.append(&mi);
        more_tile.append(&more_art);
        more_label.set_halign(Align::Center);
        more_tile.append(&more_label);
        let more = Animavel::new(&more_tile);
        more.set_visible(false);
        fixed.put(&more, 0.0, 0.0);
        let band = caixa(Orientation::Vertical, 0, "desk-banda");
        band.set_can_target(false);
        band.set_visible(false);
        fixed.put(&band, 0.0, 0.0);
        let d = Rc::new(DesktopFiles {
            area: Rc::downgrade(area), shell: Rc::downgrade(shell), fixed: fixed.clone(), window: area.window(), folder: crate::pastas::desktop(),
            icons: RefCell::default(), more, more_label, band, band_start: RefCell::default(), selected: RefCell::default(),
            anchor: RefCell::default(), memory: RefCell::new(Memory::load()), monitor: RefCell::default(), pending: StdCell::new(false),
            generation: StdCell::new(0), rename_next: RefCell::default(), renaming: RefCell::default(), drag: RefCell::default(),
            textures: RefCell::default(), last_trashed: RefCell::default(), enabled: StdCell::new(crate::tema::atual().desktop_files),
            suspended: StdCell::new(false),
        });
        // "+N": the folder in Files
        let c = gtk::GestureClick::new();
        let w = Rc::downgrade(&d);
        c.connect_released(move |_, _, _, _| {
            if let Some(d) = w.upgrade() {
                d.show_in_files(None);
            }
        });
        more_tile.add_controller(c);
        d.empty_desktop_gestures();
        d.keys();
        d.drop_target();
        d.watch();
        let w = Rc::downgrade(&d);
        crate::tema::ao_mudar(move || {
            if let Some(d) = w.upgrade() {
                let on = crate::tema::atual().desktop_files;
                if d.enabled.replace(on) != on {
                    if debug() {
                        eprintln!("debug desktop: arquivos {}", if on { "mostrados" } else { "escondidos" });
                    }
                    d.reload();
                } else {
                    // another theme: the folders' art in its colours, the names' bases on its wallpaper
                    let icons = d.icons.borrow().clone();
                    for ic in &icons {
                        d.fill_art(ic);
                    }
                    d.arrange();
                }
            }
        });
        d.reload();
        d
    }

    fn shown(&self) -> bool {
        self.enabled.get() && !self.suspended.get()
    }

    /// While the widgets are edited the icons step aside (the widgets take any cell).
    pub fn suspend(&self, on: bool) {
        self.suspended.set(on);
        self.arrange();
    }

    pub fn has_icons(&self) -> bool {
        self.enabled.get() && !self.icons.borrow().is_empty()
    }

    // ---- the folder

    fn watch(self: &Rc<Self>) {
        let mon = gio::File::for_path(&self.folder).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, None::<&gio::Cancellable>).ok();
        if let Some(m) = &mon {
            let w = Rc::downgrade(self);
            m.connect_changed(move |_, f, other, ev| {
                let Some(d) = w.upgrade() else { return };
                if ev == gio::FileMonitorEvent::Renamed {
                    // (a rename by any app keeps its place)
                    if let (Some(a), Some(b)) = (f.basename(), other.and_then(|o| o.basename())) {
                        let (a, b) = (a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned());
                        d.memory.borrow_mut().renamed(&a, &b);
                        d.memory.borrow().save();
                        let (old, new) = (d.folder.join(&a), d.folder.join(&b));
                        let sel = d.selected.borrow_mut().remove(&old);
                        if sel {
                            d.selected.borrow_mut().insert(new);
                        }
                    }
                }
                d.reload_soon();
            });
        }
        *self.monitor.borrow_mut() = mon;
    }

    fn reload_soon(self: &Rc<Self>) {
        if self.pending.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(120), move || {
            if let Some(d) = w.upgrade() {
                d.pending.set(false);
                d.reload();
            }
        });
    }

    /// Reads the folder (in the background) and brings the icons up to date.
    pub fn reload(self: &Rc<Self>) {
        let g = self.generation.get() + 1;
        self.generation.set(g);
        if !self.enabled.get() {
            self.update(Vec::new());
            return;
        }
        let p = self.folder.clone();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let Ok(rd) = std::fs::read_dir(&p) else { return Vec::new() };
            rd.filter_map(|e| e.ok()).filter(|e| !e.file_name().to_string_lossy().starts_with('.')).map(|e| arquivos::ler_um(&e.path())).collect::<Vec<Item>>()
        }, move |items| {
            if let Some(d) = w.upgrade().filter(|d| d.generation.get() == g) {
                d.update(items);
            }
        });
    }

    fn update(self: &Rc<Self>, items: Vec<Item>) {
        let mut entries: Vec<(String, bool)> = items.iter().map(|i| (i.nome.clone(), i.pasta)).collect();
        sort_entries(&mut entries);
        let by_name: HashMap<String, Item> = items.into_iter().map(|i| (i.nome.clone(), i)).collect();
        let old: Vec<Rc<Icon>> = std::mem::take(&mut *self.icons.borrow_mut());
        let mut keep: HashMap<PathBuf, Rc<Icon>> = old.iter().map(|i| (i.item.borrow().caminho.clone(), i.clone())).collect();
        let mut new_icons = Vec::new();
        let mut entering = Vec::new();
        for (name, _) in &entries {
            let Some(item) = by_name.get(name) else { continue };
            match keep.remove(&item.caminho) {
                Some(ic) => {
                    let changed = {
                        let o = ic.item.borrow();
                        o.modificado != item.modificado || o.tamanho != item.tamanho || o.emblema != item.emblema || o.pasta != item.pasta
                    };
                    if changed {
                        *ic.item.borrow_mut() = item.clone();
                        self.fill_art(&ic);
                    }
                    new_icons.push(ic);
                }
                None => {
                    let ic = self.icon(item.clone());
                    entering.push(ic.clone());
                    new_icons.push(ic);
                }
            }
        }
        // gone: out with the quick spring
        for (_, ic) in keep {
            let (f, a) = (self.fixed.clone(), ic.anim.clone());
            if ic.anim.is_visible() {
                ic.anim.sair((0.0, 0.0), 0.9, move || f.remove(&a));
            } else {
                f.remove(&a);
            }
        }
        let exists: BTreeSet<PathBuf> = new_icons.iter().map(|i| i.item.borrow().caminho.clone()).collect();
        self.selected.borrow_mut().retain(|p| exists.contains(p));
        *self.icons.borrow_mut() = new_icons;
        self.arrange();
        for ic in &entering {
            ic.anim.entrar((0.0, 8.0), 0.92);
        }
        self.marks();
        let next = self.rename_next.borrow().clone();
        if let Some(p) = next.filter(|p| exists.contains(p)) {
            *self.rename_next.borrow_mut() = None;
            *self.selected.borrow_mut() = BTreeSet::from([p.clone()]);
            *self.anchor.borrow_mut() = Some(p.clone());
            self.marks();
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(250), move || if let Some(d) = w.upgrade() { d.rename(&p) });
        }
        if debug() {
            eprintln!("debug desktop: {} ícones em {}", self.icons.borrow().len(), self.folder.display());
        }
    }

    fn grid(&self) -> Option<Grid> {
        let a = self.area.upgrade()?;
        let (w, h) = a.surface_size();
        (w > 0).then(|| Grid { width: w, height: h, obstacles: a.obstacles() })
    }

    /// Every icon to its cell (sliding from where it was).
    pub fn arrange(&self) {
        let icons = self.icons.borrow().clone();
        let Some(grid) = self.grid().filter(|_| self.shown()) else {
            for ic in &icons {
                ic.anim.set_visible(false);
            }
            self.more.set_visible(false);
            return;
        };
        let names: Vec<String> = icons.iter().map(|i| i.item.borrow().nome.clone()).collect();
        let pl = place(&grid, &names, &self.memory.borrow().positions);
        let placed: HashMap<usize, Cell> = pl.cells.iter().copied().collect();
        let area = self.area.upgrade();
        for (i, ic) in icons.iter().enumerate() {
            let Some(c) = placed.get(&i).copied() else {
                ic.anim.set_visible(false);
                ic.cell.set(None);
                continue;
            };
            let r = grid.rect(c);
            let before = ic.cell.replace(Some(c)).filter(|_| ic.anim.is_visible()).map(|b| grid.rect(b));
            ic.anim.set_visible(true);
            self.fixed.move_(&ic.anim, f64::from(r.x), f64::from(r.y));
            if let Some(b) = before.filter(|b| (b.x, b.y) != (r.x, r.y)) {
                ic.anim.definir(Canal::X, f64::from(b.x - r.x)).definir(Canal::Y, f64::from(b.y - r.y));
                ic.anim.ir(Canal::X, 0.0, mv::SUAVE).ir(Canal::Y, 0.0, mv::SUAVE);
            }
            // the name gets a base when the wallpaper under it is too light (as the widgets do)
            let base = area.as_ref().is_some_and(|a| a.wallpaper_too_light(r.x, r.y + 62, r.w, 40));
            if base { ic.label.add_css_class("com-base") } else { ic.label.remove_css_class("com-base") }
            if debug() {
                eprintln!("debug desktop: ícone {} em {} {}", ic.item.borrow().nome, r.x + r.w / 2, BARRA_ALTURA + r.y + r.h / 2);
            }
        }
        match pl.more {
            Some((c, n)) => {
                let r = grid.rect(c);
                self.more_label.set_text(&trf("{n} more", &[("n", n.to_string())]));
                self.fixed.move_(&self.more, f64::from(r.x), f64::from(r.y));
                self.more.set_visible(true);
            }
            None => self.more.set_visible(false),
        }
    }

    /// "Organizar ícones": every icon back in the order (the places the person chose are forgotten).
    pub fn auto_arrange(&self) {
        self.memory.borrow_mut().positions.clear();
        self.memory.borrow().save();
        self.arrange();
        if debug() {
            eprintln!("debug desktop: organizados");
        }
    }

    // ---- one icon

    fn icon(self: &Rc<Self>, item: Item) -> Rc<Icon> {
        let tile = caixa(Orientation::Vertical, 5, "desk-icone");
        tile.set_size_request(TILE_W, TILE_H);
        let art = caixa(Orientation::Vertical, 0, "");
        art.set_size_request(64, 56);
        art.set_halign(Align::Center);
        // (its content fills the 56 px; without this the tile's spare height under a one-line name went to the art,
        // and the icon sank)
        art.set_vexpand(false);
        tile.append(&art);
        let label = rotulo("", "desk-icone-nome");
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_lines(2);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_justify(gtk::Justification::Center);
        label.set_max_width_chars(12);
        label.set_halign(Align::Center);
        label.set_valign(Align::Start);
        tile.append(&label);
        let anim = Animavel::new(&tile);
        self.fixed.put(&anim, 0.0, 0.0);
        let ic = Rc::new(Icon { item: RefCell::new(item), anim, tile, label, art, cell: StdCell::new(None) });
        self.fill_art(&ic);
        self.clicks(&ic);
        self.drag_source(&ic);
        self.folder_target(&ic);
        ic
    }

    /// The art and the name (again, after a change).
    fn fill_art(self: &Rc<Self>, ic: &Rc<Icon>) {
        while let Some(c) = ic.art.first_child() {
            ic.art.remove(&c);
        }
        let i = ic.item.borrow().clone();
        let app = launcher(&i);
        let name = app.as_ref().map(|a| a.display_name().to_string()).unwrap_or_else(|| i.nome.clone());
        ic.label.set_text(&name);
        // the whole name, when two lines may not hold it
        ic.tile.set_tooltip_text((i.nome.chars().count() > 22 || app.is_some()).then_some(i.nome.as_str()));
        let w: gtk::Widget = if let Some(a) = &app {
            let img = a.icon().map(|g| gtk::Image::from_gicon(&g)).unwrap_or_else(|| gtk::Image::from_icon_name("application-x-executable"));
            img.set_pixel_size(48);
            img.upcast()
        } else if i.pasta {
            arquivos::arte_pasta(&i.caminho, i.emblema, 56).upcast()
        } else if arquivos::e_imagem(&i.tipo) && i.tamanho < 96 << 20 {
            let img = gtk::Image::new();
            img.set_pixel_size(56);
            img.set_size_request(56, 56);
            img.add_css_class("desk-miniatura");
            img.set_overflow(gtk::Overflow::Hidden);
            let iw = img.downgrade();
            self.texture(&i, Alvo::Foto(56, 56), move |t| if let Some(img) = iw.upgrade() { img.set_paintable(Some(t)) });
            img.upcast()
        } else {
            // a page drawn with its kind's badge, until (unless) the system's thumbnail comes; the badge stays on it
            let holder = gtk::Overlay::new();
            holder.set_halign(Align::Center);
            let fake = fake_page();
            holder.set_child(Some(&fake));
            let ext = arquivos::extensao(&i.caminho);
            if !ext.is_empty() && ext.len() <= 5 {
                let b = rotulo(&ext.to_uppercase(), "arq-ext");
                b.add_css_class("desk-selo");
                let (_, tom, _) = arquivos::glifo_do_tipo(&i);
                b.add_css_class(&format!("tom-{tom}"));
                b.set_halign(Align::Start);
                b.set_valign(Align::End);
                b.set_margin_start(5);
                b.set_margin_bottom(5);
                holder.add_overlay(&b);
            }
            let hw = holder.downgrade();
            self.texture(&i, Alvo::Pagina(44, 56), move |t| {
                let Some(h) = hw.upgrade() else { return };
                let img = gtk::Image::from_paintable(Some(t));
                img.set_pixel_size(t.width().max(t.height()));
                img.set_size_request(t.width(), t.height());
                img.set_valign(Align::Start);
                img.add_css_class("arq-pagina");
                h.set_child(Some(&img));
            });
            holder.upcast()
        };
        w.set_halign(Align::Center);
        w.set_valign(Align::Center);
        w.set_vexpand(true);
        ic.art.append(&w);
    }

    /// A thumbnail (the shared cache's, or made in the background).
    fn texture(self: &Rc<Self>, i: &Item, alvo: Alvo, show: impl Fn(&gdk::Texture) + 'static) {
        let key = (i.caminho.clone(), i.modificado, alvo);
        if let Some(t) = self.textures.borrow().get(&key).cloned() {
            show(&t);
            return;
        }
        if !arquivos::e_imagem(&i.tipo) && i.tamanho >= 512 << 20 {
            return;
        }
        let (w, p, mime, mt) = (Rc::downgrade(self), i.caminho.clone(), i.tipo.clone(), i.modificado);
        crate::lateral::em_segundo_plano(move || arquivos::miniatura(&p, &mime, mt, alvo), move |r| {
            let (Some(d), Some(r)) = (w.upgrade(), r) else { return };
            let t = arquivos::textura(r);
            d.textures.borrow_mut().insert(key, t.clone());
            show(&t);
        });
    }

    fn path_of(ic: &Icon) -> PathBuf {
        ic.item.borrow().caminho.clone()
    }

    fn clicks(self: &Rc<Self>, ic: &Rc<Icon>) {
        let c = gtk::GestureClick::new();
        c.set_button(0);
        let (w, iw) = (Rc::downgrade(self), Rc::downgrade(ic));
        c.connect_pressed(move |g, n, x, y| {
            let (Some(d), Some(ic)) = (w.upgrade(), iw.upgrade()) else { return };
            let p = Self::path_of(&ic);
            let mods = g.current_event_state();
            d.fixed.grab_focus();
            if g.current_button() == 3 {
                g.set_state(gtk::EventSequenceState::Claimed);
                if !d.selected.borrow().contains(&p) {
                    *d.selected.borrow_mut() = BTreeSet::from([p.clone()]);
                    *d.anchor.borrow_mut() = Some(p);
                    d.marks();
                }
                d.menu(&ic, x, y);
                return;
            }
            if g.current_button() != 1 {
                return;
            }
            if n >= 2 {
                let w = Rc::downgrade(&d);
                glib::idle_add_local_once(move || if let Some(d) = w.upgrade() { d.open_selection() });
                return;
            }
            if mods.contains(gdk::ModifierType::CONTROL_MASK) {
                let had = d.selected.borrow_mut().remove(&p);
                if !had {
                    d.selected.borrow_mut().insert(p.clone());
                }
                *d.anchor.borrow_mut() = Some(p);
            } else if mods.contains(gdk::ModifierType::SHIFT_MASK) {
                let order: Vec<PathBuf> = d.icons.borrow().iter().filter(|i| i.cell.get().is_some()).map(|i| Self::path_of(i)).collect();
                let a = d.anchor.borrow().clone().and_then(|a| order.iter().position(|q| *q == a)).unwrap_or(0);
                let b = order.iter().position(|q| *q == p).unwrap_or(0);
                *d.selected.borrow_mut() = order[a.min(b)..=a.max(b)].iter().cloned().collect();
            } else if !d.selected.borrow().contains(&p) {
                *d.selected.borrow_mut() = BTreeSet::from([p.clone()]);
                *d.anchor.borrow_mut() = Some(p);
            }
            d.marks();
        });
        // a click (not a drag) on one of several chosen: only it
        let (w, iw) = (Rc::downgrade(self), Rc::downgrade(ic));
        c.connect_released(move |g, n, _, _| {
            let (Some(d), Some(ic)) = (w.upgrade(), iw.upgrade()) else { return };
            let mods = g.current_event_state();
            if g.current_button() != 1 || n != 1 || mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK) {
                return;
            }
            let p = Self::path_of(&ic);
            if d.selected.borrow().len() > 1 {
                *d.selected.borrow_mut() = BTreeSet::from([p.clone()]);
                *d.anchor.borrow_mut() = Some(p);
                d.marks();
            }
        });
        ic.tile.add_controller(c);
    }

    /// The chosen ones marked (and nothing else).
    fn marks(&self) {
        let sel = self.selected.borrow();
        for ic in self.icons.borrow().iter() {
            if sel.contains(&ic.item.borrow().caminho) {
                ic.tile.add_css_class("escolhido");
            } else {
                ic.tile.remove_css_class("escolhido");
            }
        }
        if debug() {
            eprintln!("debug desktop: {} escolhidos", sel.len());
        }
    }

    fn selection(&self) -> Vec<PathBuf> {
        self.selected.borrow().iter().cloned().collect()
    }

    // ---- the empty desktop: a click clears, a drag is a rubber band

    fn empty_desktop_gestures(self: &Rc<Self>) {
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let w = Rc::downgrade(self);
        drag.connect_drag_begin(move |g, x, y| {
            let Some(d) = w.upgrade() else { return };
            let fixed: gtk::Widget = d.fixed.clone().upcast();
            // (on an icon is not empty: its own gestures have it; the pick alone did not see through the icons' motion)
            let empty = d.fixed.pick(x, y, gtk::PickFlags::DEFAULT).is_none_or(|p| p == fixed) && !d.icon_at(x, y);
            if !empty || !d.shown() {
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            d.fixed.grab_focus();
            d.finish_rename(true);
            let keep = if g.current_event_state().intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK) {
                d.selected.borrow().clone()
            } else {
                BTreeSet::new()
            };
            *d.band_start.borrow_mut() = Some((x, y, keep));
        });
        let w = Rc::downgrade(self);
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(d) = w.upgrade() else { return };
            let Some((x, y, keep)) = d.band_start.borrow().clone() else { return };
            if dx.abs() < 3.0 && dy.abs() < 3.0 {
                return;
            }
            let r = Rect::from_corners((x, y), (x + dx, y + dy));
            d.band.set_size_request(r.w.max(1), r.h.max(1));
            d.fixed.move_(&d.band, f64::from(r.x), f64::from(r.y));
            d.band.set_visible(true);
            let grid = d.grid();
            let mut sel = keep;
            if let Some(grid) = grid {
                for ic in d.icons.borrow().iter() {
                    if ic.cell.get().is_some_and(|c| grid.rect(c).overlaps(&r)) {
                        sel.insert(Self::path_of(ic));
                    }
                }
            }
            if *d.selected.borrow() != sel {
                *d.selected.borrow_mut() = sel;
                d.marks();
            }
        });
        let w = Rc::downgrade(self);
        drag.connect_drag_end(move |_, dx, dy| {
            let Some(d) = w.upgrade() else { return };
            let start = d.band_start.borrow_mut().take();
            d.band.set_visible(false);
            // a click: nothing chosen anymore
            if let Some((_, _, keep)) = start {
                if dx.abs() < 3.0 && dy.abs() < 3.0 && *d.selected.borrow() != keep {
                    *d.selected.borrow_mut() = keep;
                    d.marks();
                }
            }
        });
        self.fixed.add_controller(drag);
        self.fixed.set_focusable(true);
    }

    /// Is there an icon (or "+N") at this point of the desktop?
    pub(crate) fn icon_at(&self, x: f64, y: f64) -> bool {
        let Some(grid) = self.grid() else { return false };
        let p = Rect::new(x as i32, y as i32, 1, 1);
        self.icons.borrow().iter().filter_map(|i| i.cell.get()).any(|c| grid.rect(c).overlaps(&p))
            || (self.more.is_visible() && self.more.compute_bounds(&self.fixed).is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32))))
    }

    // ---- the keyboard

    fn keys(self: &Rc<Self>) {
        let k = gtk::EventControllerKey::new();
        let w = Rc::downgrade(self);
        k.connect_key_pressed(move |_, key, _, mods| {
            let Some(d) = w.upgrade() else { return glib::Propagation::Proceed };
            d.key(key, mods)
        });
        self.fixed.add_controller(k);
    }

    fn key(self: &Rc<Self>, key: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        use gdk::Key;
        if !self.shown() || self.renaming.borrow().is_some() {
            return glib::Propagation::Proceed;
        }
        // (only when the desktop itself has the keyboard: the ask field's keys are its own)
        let fixed: gtk::Widget = self.fixed.clone().upcast();
        if gtk::prelude::GtkWindowExt::focus(&self.window).is_some_and(|f| f != fixed) {
            return glib::Propagation::Proceed;
        }
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let sel = self.selection();
        match key {
            Key::Left | Key::Right | Key::Up | Key::Down => {
                let dir = match key { Key::Left => Dir::Left, Key::Right => Dir::Right, Key::Up => Dir::Up, _ => Dir::Down };
                self.step(dir, shift);
            }
            Key::Home | Key::End => {
                let shown: Vec<PathBuf> = self.icons.borrow().iter().filter(|i| i.cell.get().is_some()).map(|i| Self::path_of(i)).collect();
                if let Some(p) = if key == Key::Home { shown.first() } else { shown.last() } {
                    *self.selected.borrow_mut() = BTreeSet::from([p.clone()]);
                    *self.anchor.borrow_mut() = Some(p.clone());
                    self.marks();
                }
            }
            Key::Return | Key::KP_Enter => self.open_selection(),
            Key::F2 => {
                if let [p] = sel.as_slice() {
                    self.rename(p);
                }
            }
            Key::Delete | Key::KP_Delete => self.trash(&sel),
            Key::Escape => {
                self.selected.borrow_mut().clear();
                self.marks();
            }
            Key::a if ctrl => {
                *self.selected.borrow_mut() = self.icons.borrow().iter().filter(|i| i.cell.get().is_some()).map(|i| Self::path_of(i)).collect();
                self.marks();
            }
            Key::c | Key::x if ctrl => self.copy(&sel, key == Key::x),
            Key::v if ctrl => self.paste(),
            Key::z if ctrl => self.undo_trash(),
            Key::N if ctrl => self.new_folder(None),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    fn step(&self, dir: Dir, extend: bool) {
        let icons = self.icons.borrow().clone();
        let shown: Vec<(PathBuf, Cell)> = icons.iter().filter_map(|i| i.cell.get().map(|c| (Self::path_of(i), c))).collect();
        if shown.is_empty() {
            return;
        }
        let from = self.anchor.borrow().clone().and_then(|a| shown.iter().find(|(p, _)| *p == a).map(|(_, c)| *c));
        let to = match from {
            Some(c) => neighbour(&shown.iter().map(|(_, c)| *c).collect::<Vec<_>>(), c, dir),
            None => Some(0),
        };
        let Some(k) = to else { return };
        let p = shown[k].0.clone();
        if extend {
            self.selected.borrow_mut().insert(p.clone());
        } else {
            *self.selected.borrow_mut() = BTreeSet::from([p.clone()]);
        }
        *self.anchor.borrow_mut() = Some(p);
        self.marks();
    }

    // ---- what can be done with them

    fn open_selection(self: &Rc<Self>) {
        for p in self.selection().into_iter().take(12) {
            self.open(&p);
        }
    }

    fn open(self: &Rc<Self>, p: &Path) {
        let Some(s) = self.shell.upgrade() else { return };
        if p.is_dir() {
            s.abrir_arquivos(Some(p));
            return;
        }
        if p.extension().is_some_and(|e| e == "kalango-agente") {
            s.instalar_agente(p);
            return;
        }
        // a launcher runs only when it is marked executable (as other desktops ask): else it is a text file
        if p.extension().is_some_and(|e| e == "desktop") && executable(p) {
            if let Some(a) = gio::DesktopAppInfo::from_filename(p) {
                let ctx = WidgetExt::display(&self.window).app_launch_context();
                if let Err(e) = crate::apps::launch_with(a.upcast_ref(), &[], Some(ctx.upcast_ref())) {
                    eprintln!("kalango-shell: área de trabalho: {e}");
                }
                if debug() {
                    eprintln!("debug desktop: aberto {}", p.display());
                }
                return;
            }
        }
        let f = gio::File::for_path(p);
        crate::apps::launch_file(&f, Some(self.window.upcast_ref()));
        gtk::RecentManager::default().add_item(&f.uri());
        if debug() {
            eprintln!("debug desktop: aberto {}", p.display());
        }
    }

    fn show_in_files(&self, p: Option<&Path>) {
        if let Some(s) = self.shell.upgrade() {
            s.abrir_arquivos(Some(p.unwrap_or(&self.folder)));
        }
    }

    fn copy(&self, sel: &[PathBuf], cut: bool) {
        if sel.is_empty() {
            return;
        }
        arquivos::remember_cut(sel, cut);
        let files: Vec<gio::File> = sel.iter().map(gio::File::for_path).collect();
        self.window.clipboard().set_content(Some(&gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()))).ok();
        if debug() {
            eprintln!("debug desktop: {} {}", if cut { "recortados" } else { "copiados" }, sel.len());
        }
    }

    /// What is on the clipboard into the Desktop folder (moved when it was cut).
    pub fn paste(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.window.clipboard().read_value_async(gdk::FileList::static_type(), glib::Priority::DEFAULT, None::<&gio::Cancellable>, move |r| {
            let (Some(d), Ok(v)) = (w.upgrade(), r) else { return };
            let Ok(fl) = v.get::<gdk::FileList>() else { return };
            let ps: Vec<PathBuf> = fl.files().iter().filter_map(|f| f.path()).collect();
            let cut = arquivos::was_cut(&ps);
            let folder = d.folder.clone();
            d.put_into(ps, folder, cut);
        });
    }

    /// Copies or moves files into a folder: a long one goes to the island (design "Operação de arquivos"), a short
    /// one is done at once; a name already there keeps both ("x (2)").
    fn put_into(self: &Rc<Self>, ps: Vec<PathBuf>, dest: PathBuf, mover: bool) {
        let ps: Vec<PathBuf> = ps.into_iter().filter(|p| *p != dest && !dest.starts_with(p) && (!mover || p.parent() != Some(dest.as_path()))).collect();
        if ps.is_empty() {
            return;
        }
        let shell = self.shell.clone();
        crate::lateral::em_segundo_plano(move || {
            use std::os::unix::fs::MetadataExt;
            let pr = crate::operacao::Progresso::default();
            for p in &ps {
                crate::operacao::contar(p, &pr);
            }
            let disk = |p: &Path| std::fs::metadata(p).ok().map(|m| m.dev());
            let rename = mover && ps.iter().all(|p| disk(p) == disk(&dest));
            let long = !rename && (pr.total_bytes.load(std::sync::atomic::Ordering::Relaxed) > 64 << 20 || pr.total_itens.load(std::sync::atomic::Ordering::Relaxed) > 300);
            if long {
                return (ps, dest, true, 0);
            }
            let mut done = 0;
            let _ = std::fs::create_dir_all(&dest);
            for p in &ps {
                let Some(n) = p.file_name() else { continue };
                let to = crate::operacao::nome_livre(&dest, &n.to_string_lossy());
                let r = if mover {
                    std::fs::rename(p, &to).or_else(|_| arquivos::copiar(p, &to).and_then(|_| if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }))
                } else {
                    arquivos::copiar(p, &to)
                };
                match r {
                    Ok(()) => done += 1,
                    Err(e) => eprintln!("kalango-shell: área de trabalho: {}: {e}", p.display()),
                }
            }
            (Vec::new(), dest, false, done)
        }, move |(ps, dest, long, done)| {
            if long {
                if let Some(ops) = shell.upgrade().and_then(|s| s.operacoes()) {
                    ops.iniciar(ps, dest.clone(), mover);
                }
            }
            if debug() {
                eprintln!("debug desktop: {} {done} em {}", if mover { "movidos" } else { "copiados" }, dest.display());
            }
        });
    }

    fn trash(self: &Rc<Self>, sel: &[PathBuf]) {
        if sel.is_empty() {
            return;
        }
        let mut done = Vec::new();
        for p in sel {
            match arquivos::para_lixeira(p) {
                Ok(t) => done.push((p.clone(), t)),
                Err(e) => eprintln!("kalango-shell: lixeira {}: {e}", p.display()),
            }
        }
        if done.is_empty() {
            return;
        }
        let title = if done.len() == 1 {
            trf("“{name}” moved to the trash", &[("name", done[0].0.file_name().unwrap_or_default().to_string_lossy().into_owned())])
        } else {
            trf("{n} items moved to the trash", &[("n", done.len().to_string())])
        };
        *self.last_trashed.borrow_mut() = done;
        self.selected.borrow_mut().clear();
        if debug() {
            eprintln!("debug desktop: lixeira {}", self.last_trashed.borrow().len());
        }
        if let Some(s) = self.shell.upgrade() {
            let w = Rc::downgrade(self);
            s.notificacoes().notify_local_acoes(tr("Desktop"), &title, tr("Ctrl+Z on the desktop also brings it back."), &[("desfazer", tr("Undo"))], move |k| {
                if k == "desfazer" {
                    if let Some(d) = w.upgrade() {
                        d.undo_trash();
                    }
                }
            });
        }
    }

    /// What went to the trash from here last comes back (to its place: the name keeps its cell).
    pub fn undo_trash(self: &Rc<Self>) {
        let last = std::mem::take(&mut *self.last_trashed.borrow_mut());
        let mut n = 0;
        for (_, t) in last.iter().filter(|(_, t)| !t.as_os_str().is_empty()) {
            match arquivos::restaurar(t) {
                Ok(p) => {
                    n += 1;
                    self.selected.borrow_mut().insert(p);
                }
                Err(e) => eprintln!("kalango-shell: restaurar {}: {e}", t.display()),
            }
        }
        if debug() {
            eprintln!("debug desktop: restaurados {n}");
        }
    }

    /// "Nova pasta": in the Desktop folder, at the cell under the pointer when there is one; named in place.
    pub fn new_folder(self: &Rc<Self>, at: Option<(f64, f64)>) {
        let p = crate::operacao::nome_livre(&self.folder, tr("New folder"));
        if let Err(e) = std::fs::create_dir_all(&p) {
            eprintln!("kalango-shell: nova pasta: {e}");
            return;
        }
        if debug() {
            eprintln!("debug area: nova pasta {}", p.display());
        }
        if !self.shown() {
            self.show_in_files(Some(&p));
            return;
        }
        if let (Some((x, y)), Some(grid)) = (at, self.grid()) {
            let taken: HashSet<Cell> = self.icons.borrow().iter().filter_map(|i| i.cell.get()).collect();
            if let Some(c) = grid.nearest_free(grid.snap(x as i32 - TILE_W / 2, y as i32 - TILE_H / 3), &taken) {
                if let Some(n) = p.file_name() {
                    self.memory.borrow_mut().put(&n.to_string_lossy(), c);
                    self.memory.borrow().save();
                }
            }
        }
        *self.rename_next.borrow_mut() = Some(p);
        self.reload_soon();
    }

    /// F2: the name in a field over the icon's (the part before the extension chosen); Enter or a click elsewhere
    /// keeps it, Esc goes back.
    fn rename(self: &Rc<Self>, p: &Path) {
        self.finish_rename(true);
        let Some(ic) = self.icons.borrow().iter().find(|i| i.item.borrow().caminho == p).cloned() else { return };
        let (Some(c), Some(grid)) = (ic.cell.get(), self.grid()) else { return };
        let r = grid.rect(c);
        let e = gtk::Entry::new();
        e.add_css_class("desk-renomear");
        let name = ic.item.borrow().nome.clone();
        e.set_text(&name);
        e.set_width_chars(1);
        e.set_size_request(TILE_W + 48, -1);
        self.fixed.put(&e, f64::from(r.x - 24), f64::from(r.y + 64));
        ic.label.set_opacity(0.0);
        *self.renaming.borrow_mut() = Some((p.to_path_buf(), e.clone()));
        let w = Rc::downgrade(self);
        e.connect_activate(move |_| if let Some(d) = w.upgrade() { d.finish_rename(true) });
        let k = gtk::EventControllerKey::new();
        let w = Rc::downgrade(self);
        k.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(d) = w.upgrade() {
                    d.finish_rename(false);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        e.add_controller(k);
        let f = gtk::EventControllerFocus::new();
        let w = Rc::downgrade(self);
        f.connect_leave(move |_| {
            let w = w.clone();
            glib::idle_add_local_once(move || if let Some(d) = w.upgrade() { d.finish_rename(true) });
        });
        e.add_controller(f);
        crate::superficies::take_keyboard(&self.window);
        e.grab_focus();
        let end = name.rfind('.').filter(|i| *i > 0 && !p.is_dir()).map(|i| name[..i].chars().count() as i32).unwrap_or(-1);
        e.select_region(0, end);
        if debug() {
            eprintln!("debug desktop: renomeando {name}");
        }
    }

    fn finish_rename(self: &Rc<Self>, keep: bool) {
        let Some((p, e)) = self.renaming.borrow_mut().take() else { return };
        let new = e.text().trim().to_string();
        self.fixed.remove(&e);
        gtk4_layer_shell::LayerShell::set_keyboard_mode(&self.window, gtk4_layer_shell::KeyboardMode::OnDemand);
        if let Some(ic) = self.icons.borrow().iter().find(|i| i.item.borrow().caminho == p) {
            ic.label.set_opacity(1.0);
        }
        self.fixed.grab_focus();
        let old = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !keep || new.is_empty() || new == old || new.contains('/') || new == "." || new == ".." {
            return;
        }
        let to = p.with_file_name(&new);
        if to.exists() {
            eprintln!("kalango-shell: renomear: {} já existe", to.display());
            return;
        }
        match std::fs::rename(&p, &to) {
            Ok(()) => {
                self.memory.borrow_mut().renamed(&old, &new);
                self.memory.borrow().save();
                let was = self.selected.borrow_mut().remove(&p);
                if was {
                    self.selected.borrow_mut().insert(to.clone());
                }
                *self.anchor.borrow_mut() = Some(to);
                if debug() {
                    eprintln!("debug desktop: renomeado {new}");
                }
            }
            Err(e) => eprintln!("kalango-shell: renomear: {e}"),
        }
    }

    /// "Pedir ao agente": a new conversation in the agents center, the file(s) already in what is being written.
    fn ask_agent(&self, ps: &[PathBuf]) {
        let Some(s) = self.shell.upgrade() else { return };
        s.abrir_central(Some("conversas"));
        if let Some(c) = s.central() {
            let list = ps.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
            c.conversas().preencher(&if ps.len() == 1 { trf("About the file {path}: ", &[("path", list)]) } else { trf("About the files {paths}: ", &[("paths", list)]) });
        }
    }

    /// The menu of an icon (design "Área de trabalho · menu de um arquivo").
    fn menu(self: &Rc<Self>, ic: &Rc<Icon>, x: f64, y: f64) {
        use crate::context_menu::Menu;
        let m = Menu::at(&ic.tile, x, y);
        let c = m.main.clone();
        let sel = self.selection();
        let one = (sel.len() == 1).then(|| sel[0].clone());
        let folder = one.as_ref().is_some_and(|p| p.is_dir());
        let w = Rc::downgrade(self);
        let act = move |f: Box<dyn Fn(&Rc<DesktopFiles>)>| {
            let w = w.clone();
            move || {
                if let Some(d) = w.upgrade() {
                    f(&d);
                }
            }
        };
        let open = m.item(&c, "pasta", tr("Open"), "↵", "", act(Box::new(|d| d.open_selection())));
        crate::central::posicao_debug("area arquivo menu abrir", open.upcast_ref());
        if let (Some(p), false) = (one.clone(), folder) {
            let page = m.sub(&c, "apps-n", tr("Open with"), "abrir-com");
            let item = arquivos::ler_um(&p);
            let (apps, default) = arquivos::apps_para(&item.tipo);
            for a in apps.iter().take(5) {
                let (a2, pp, win) = (a.clone(), p.clone(), self.window.clone());
                let is_default = default.as_ref().is_some_and(|d| d.id() == a.id());
                m.item(&page, "apps-n", &a.display_name(), if is_default { tr("default") } else { "" }, "", move || {
                    let ctx = WidgetExt::display(&win).app_launch_context();
                    if let Err(e) = crate::apps::launch_with(&a2, &[gio::File::for_path(&pp)], Some(ctx.upcast_ref())) {
                        eprintln!("kalango-shell: abrir com: {e}");
                    }
                });
            }
            let (s, pp) = (self.shell.clone(), p.clone());
            m.item(&page, "mais", tr("Another app…"), "", "", move || if let Some(s) = s.upgrade() { s.abrir_com(&pp) });
        }
        Menu::divider(&c);
        let s2 = sel.clone();
        m.item(&c, "copiar", tr("Copy"), "Ctrl+C", "", act(Box::new(move |d| d.copy(&s2, false))));
        let s2 = sel.clone();
        m.item(&c, "tesoura", tr("Cut"), "Ctrl+X", "", act(Box::new(move |d| d.copy(&s2, true))));
        let o2 = one.clone();
        let ren = m.item(&c, "lapis", tr("Rename"), "F2", "", act(Box::new(move |d| if let Some(p) = &o2 { d.rename(p) })));
        ren.set_sensitive(one.is_some());
        crate::central::posicao_debug("area arquivo menu renomear", ren.upcast_ref());
        Menu::divider(&c);
        let first = sel.first().cloned();
        let show = m.item(&c, "pasta", tr("Show in Files"), "", "", act(Box::new(move |d| d.show_in_files(first.as_deref()))));
        crate::central::posicao_debug("area arquivo menu mostrar", show.upcast_ref());
        let s2 = sel.clone();
        m.item(&c, "spark", tr("Ask the agent"), "", "agente", act(Box::new(move |d| d.ask_agent(&s2))));
        Menu::divider(&c);
        let s2 = sel.clone();
        let lixo = m.item(&c, "lixeira", tr("Move to the trash"), "Del", "perigo", act(Box::new(move |d| d.trash(&s2))));
        crate::central::posicao_debug("area arquivo menu lixeira", lixo.upcast_ref());
        m.show();
        if debug() {
            eprintln!("debug desktop: menu de {} arquivos", sel.len());
        }
    }

    // ---- drag and drop

    fn drag_source(self: &Rc<Self>, ic: &Rc<Icon>) {
        let ds = gtk::DragSource::new();
        ds.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
        let (w, iw) = (Rc::downgrade(self), Rc::downgrade(ic));
        ds.connect_prepare(move |_, x, y| {
            let (d, ic) = (w.upgrade()?, iw.upgrade()?);
            if d.renaming.borrow().is_some() {
                return None;
            }
            let p = Self::path_of(&ic);
            let sel = d.selection();
            let paths: Vec<PathBuf> = if sel.contains(&p) { sel } else { vec![p.clone()] };
            let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
            *d.drag.borrow_mut() = Some(OwnDrag { primary: p, paths, offset: (x, y) });
            Some(gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()))
        });
        let iw = Rc::downgrade(ic);
        ds.connect_drag_begin(move |s, _| {
            if let Some(ic) = iw.upgrade() {
                let pt = gtk::WidgetPaintable::new(Some(&ic.tile));
                s.set_icon(Some(&pt), TILE_W / 2, TILE_H / 3);
                ic.tile.add_css_class("arrastando");
            }
        });
        let (w, iw) = (Rc::downgrade(self), Rc::downgrade(ic));
        ds.connect_drag_end(move |_, _, _| {
            if let Some(ic) = iw.upgrade() {
                ic.tile.remove_css_class("arrastando");
            }
            let w = w.clone();
            glib::idle_add_local_once(move || if let Some(d) = w.upgrade() { *d.drag.borrow_mut() = None });
        });
        ic.tile.add_controller(ds);
    }

    /// A folder's icon takes dropped files (moved into it; copied from another disk or with Ctrl).
    fn folder_target(self: &Rc<Self>, ic: &Rc<Icon>) {
        let t = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::MOVE | gdk::DragAction::COPY);
        let (w, iw) = (Rc::downgrade(self), Rc::downgrade(ic));
        t.connect_accept(move |_, _| iw.upgrade().is_some_and(|ic| ic.item.borrow().pasta) && w.upgrade().is_some());
        let iw = Rc::downgrade(ic);
        t.connect_enter(move |_, _, _| {
            if let Some(ic) = iw.upgrade() {
                ic.tile.add_css_class("alvo");
            }
            gdk::DragAction::MOVE
        });
        let iw = Rc::downgrade(ic);
        t.connect_leave(move |_| if let Some(ic) = iw.upgrade() { ic.tile.remove_css_class("alvo") });
        let (w, iw) = (Rc::downgrade(self), Rc::downgrade(ic));
        t.connect_drop(move |t, v, _, _| {
            let (Some(d), Some(ic), Ok(fl)) = (w.upgrade(), iw.upgrade(), v.get::<gdk::FileList>()) else { return false };
            ic.tile.remove_css_class("alvo");
            let dest = Self::path_of(&ic);
            let ps: Vec<PathBuf> = fl.files().iter().filter_map(|f| f.path()).filter(|p| *p != dest).collect();
            if ps.is_empty() {
                return false;
            }
            let mover = arquivos::mover_ao_soltar(t, &ps, &dest);
            if debug() {
                eprintln!("debug desktop: soltos {} em {}", ps.len(), dest.display());
            }
            d.put_into(ps, dest, mover);
            true
        });
        ic.tile.add_controller(t);
    }

    /// The desktop takes dropped files: its own icons go to the cell they were dropped on (the others keep their
    /// places around it); files from elsewhere move into the Desktop folder, there.
    fn drop_target(self: &Rc<Self>) {
        let t = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::MOVE | gdk::DragAction::COPY);
        let w = Rc::downgrade(self);
        t.connect_accept(move |_, _| w.upgrade().is_some_and(|d| d.shown()));
        let w = Rc::downgrade(self);
        t.connect_drop(move |t, v, x, y| {
            let (Some(d), Ok(fl)) = (w.upgrade(), v.get::<gdk::FileList>()) else { return false };
            let Some(grid) = d.grid() else { return false };
            let ps: Vec<PathBuf> = fl.files().iter().filter_map(|f| f.path()).collect();
            let own = d.drag.borrow_mut().take();
            let (offset, primary) = match &own {
                Some(o) if o.paths == ps => (o.offset, Some(o.primary.clone())),
                _ => ((f64::from(TILE_W) / 2.0, f64::from(TILE_H) / 3.0), None),
            };
            let target = grid.snap((x - offset.0) as i32, (y - offset.1) as i32);
            let here = ps.iter().all(|p| p.parent() == Some(d.folder.as_path()));
            // the cells: the one under the pointer's for the first, then each one's offset from it (or the nearest free)
            let icons = d.icons.borrow().clone();
            let cell_of = |p: &Path| icons.iter().find(|i| i.item.borrow().caminho == p).and_then(|i| i.cell.get());
            let moving: HashSet<PathBuf> = ps.iter().cloned().collect();
            let mut taken: HashSet<Cell> = icons.iter().filter(|i| !moving.contains(&Self::path_of(i))).filter_map(|i| i.cell.get()).collect();
            let base = primary.as_deref().and_then(cell_of);
            let mut order = ps.clone();
            if let Some(p) = &primary {
                order.retain(|q| q != p);
                order.insert(0, p.clone());
            }
            for (k, p) in order.iter().enumerate() {
                let want = match (base, cell_of(p)) {
                    (Some(b), Some(c)) => Cell::new(target.col + c.col - b.col, target.row + c.row - b.row),
                    _ => Cell::new(target.col, target.row + k as i32),
                };
                let Some(c) = grid.nearest_free(want, &taken) else { continue };
                taken.insert(c);
                if let Some(n) = p.file_name() {
                    // (a file from elsewhere is named as it is when it gets here — a taken name gets "(2)" and its own place)
                    if here || !d.folder.join(n).exists() {
                        d.memory.borrow_mut().put(&n.to_string_lossy(), c);
                    }
                }
            }
            d.memory.borrow().save();
            if here {
                d.arrange();
                if debug() {
                    eprintln!("debug desktop: {} ícones movidos para {} {}", ps.len(), target.col, target.row);
                }
            } else {
                let mover = arquivos::mover_ao_soltar(t, &ps, &d.folder);
                let folder = d.folder.clone();
                if debug() {
                    eprintln!("debug desktop: soltos {} na área de trabalho", ps.len());
                }
                d.put_into(ps, folder, mover);
            }
            true
        });
        self.fixed.add_controller(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_hd() -> Grid {
        // the desktop under the 40 px top bar, with the design's widgets (widgets::padrao): the clock and the summary
        // at the left, the agents, "Pergunte" and "Agora" at the right (y on the surface)
        let b = 40;
        Grid {
            width: 1920,
            height: 1040,
            obstacles: vec![
                Rect::new(48, 64 - b, 168, 188),
                Rect::new(48, 300 - b, 352, 352),
                Rect::new(1920 - 48 - 352, 72 - b, 352, 168),
                Rect::new(1920 - 48 - 352, 256 - b, 352, 168),
                Rect::new(1920 - 48 - 352, 440 - b, 352, 168),
            ],
        }
    }

    /// Icons never sit under a widget, start beside the right-hand widgets at the top, and go down then left.
    #[test]
    fn icons_keep_clear_of_the_widgets() {
        let g = full_hd();
        let order = g.order();
        assert!(!order.is_empty());
        for c in &order {
            let r = g.rect(*c);
            assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= g.width && r.y + r.h <= g.height, "{c:?} {r:?}");
            for o in &g.obstacles {
                assert!(!o.grown(CLEAR).overlaps(&r), "{c:?} under {o:?}");
            }
        }
        // the first: the top of the first column left of the right-hand widgets
        let first = g.rect(order[0]);
        assert_eq!(order[0].row, 0);
        assert!(first.x + first.w <= 1920 - 48 - 352 - CLEAR);
        assert!(first.x + first.w > 1920 - 48 - 352 - CLEAR - STEP_X);
        // down the column first
        assert_eq!(order[1], Cell::new(order[0].col, 1));
        // the columns under the right-hand widgets come last
        assert!(order.last().unwrap().col < order[0].col);
    }

    /// No widget on the right (a short screen shows only the clock): the icons start at the right edge.
    #[test]
    fn without_right_widgets_icons_start_at_the_edge() {
        let g = Grid { width: 1366, height: 728, obstacles: vec![Rect::new(48, 24, 168, 188)] };
        let o = g.order();
        assert_eq!(o[0], Cell::new(0, 0));
        assert_eq!(g.rect(o[0]).x + TILE_W, 1366 - EDGE);
        assert_eq!(g.rows(), 6);
        // cells beside the clock are skipped
        assert!(o.iter().all(|c| !g.rect(*c).overlaps(&Rect::new(48, 24, 168, 188))));
    }

    /// A vertical screen: the narrow middle is used first, then below the widgets.
    #[test]
    fn a_vertical_screen_has_room() {
        let b = 40;
        let g = Grid {
            width: 1080,
            height: 1880,
            obstacles: vec![Rect::new(48, 64 - b, 168, 188), Rect::new(48, 300 - b, 352, 352), Rect::new(1080 - 400, 72 - b, 352, 168),
                Rect::new(1080 - 400, 256 - b, 352, 168), Rect::new(1080 - 400, 440 - b, 352, 168)],
        };
        let o = g.order();
        assert!(o.len() > 40, "{}", o.len());
        let r = g.rect(o[0]);
        assert!(r.x >= 400 + CLEAR && r.x + r.w <= 680 - CLEAR, "{r:?}");
    }

    /// Remembered places win (when free); the rest follow the order; a widget over a remembered cell sends it back.
    #[test]
    fn placement_remembers_and_falls_back() {
        let g = full_hd();
        let names: Vec<String> = ["Projetos", "Fotos", "nota.txt"].iter().map(|s| s.to_string()).collect();
        let mut mem = BTreeMap::new();
        mem.insert("nota.txt".to_string(), Cell::new(0, 7));
        let p = place(&g, &names, &mem);
        assert_eq!(p.more, None);
        assert_eq!(p.cells.len(), 3);
        assert_eq!(p.cells[2], (2, Cell::new(0, 7)));
        let order = g.order();
        assert_eq!(p.cells[0], (0, order[0]));
        assert_eq!(p.cells[1], (1, order[1]));
        // a cell under the "Agora" widget is not free: back to the order
        mem.insert("Fotos".to_string(), Cell::new(1, 1));
        let p = place(&g, &names, &mem);
        assert_eq!(p.cells[1].1, order[1]);
        // two names on one cell: the first in the order keeps it
        mem.insert("Projetos".to_string(), Cell::new(0, 7));
        let p = place(&g, &names, &mem);
        assert_eq!(p.cells[0].1, Cell::new(0, 7));
        assert_ne!(p.cells[2].1, Cell::new(0, 7));
        let all: HashSet<Cell> = p.cells.iter().map(|c| c.1).collect();
        assert_eq!(all.len(), 3);
    }

    /// More files than cells: the last cell is "+N", nothing overlaps.
    #[test]
    fn overflow_becomes_more() {
        let g = Grid { width: 400, height: 300, obstacles: vec![] };
        let free = g.order().len();
        assert_eq!(free, 6);
        let names: Vec<String> = (0..10).map(|i| format!("f{i}")).collect();
        let p = place(&g, &names, &BTreeMap::new());
        assert_eq!(p.cells.len(), 5);
        assert_eq!(p.more, Some((g.order()[5], 5)));
    }

    #[test]
    fn snapping_and_nearest_free() {
        let g = full_hd();
        let c = Cell::new(5, 3);
        let r = g.rect(c);
        assert_eq!(g.snap(r.x + 20, r.y - 30), c);
        assert_eq!(g.snap(-500, -500), Cell::new(g.cols() - 1, 0));
        let taken = HashSet::from([c]);
        let n = g.nearest_free(c, &taken).unwrap();
        assert_ne!(n, c);
        assert!((n.col - c.col).abs() + (n.row - c.row).abs() == 1, "{n:?}");
        // under a widget: the nearest free cell
        let under = Cell::new(1, 1);
        assert!(!g.free(under));
        assert!(g.free(g.nearest_free(under, &HashSet::new()).unwrap()));
    }

    #[test]
    fn folders_first_then_names_as_people_read_them() {
        let mut v: Vec<(String, bool)> = [("foto 10.png", false), ("Zeta", true), ("foto 2.png", false), ("alfa", true), ("Beta.txt", false)]
            .iter().map(|(n, d)| (n.to_string(), *d)).collect();
        sort_entries(&mut v);
        let names: Vec<&str> = v.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["alfa", "Zeta", "Beta.txt", "foto 2.png", "foto 10.png"]);
        assert_eq!(natural_cmp("Foto 2", "foto 10"), Ordering::Less);
        assert_eq!(natural_cmp("b", "A"), Ordering::Greater);
    }

    #[test]
    fn arrows_go_to_the_nearest_icon_that_way() {
        let cells = [Cell::new(0, 0), Cell::new(0, 1), Cell::new(1, 0), Cell::new(2, 1)];
        assert_eq!(neighbour(&cells, cells[0], Dir::Down), Some(1));
        assert_eq!(neighbour(&cells, cells[0], Dir::Left), Some(2));
        assert_eq!(neighbour(&cells, cells[0], Dir::Right), None);
        assert_eq!(neighbour(&cells, cells[1], Dir::Left), Some(3));
        assert_eq!(neighbour(&cells, cells[3], Dir::Right), Some(1));
        assert_eq!(neighbour(&cells, cells[2], Dir::Up), None);
    }

    #[test]
    fn positions_are_remembered_by_name() {
        let mut m = Memory::default();
        m.put("a", Cell::new(0, 0));
        m.put("b", Cell::new(0, 0));
        assert_eq!(m.positions.len(), 1, "a cell holds one icon");
        m.renamed("b", "c");
        assert_eq!(m.positions.get("c"), Some(&Cell::new(0, 0)));
        let s = serde_json::to_vec(&m).unwrap();
        assert_eq!(serde_json::from_slice::<Memory>(&s).unwrap(), m);
        assert_eq!(serde_json::from_slice::<Memory>(b"{}").unwrap(), Memory::default());
    }

    #[test]
    fn a_rubber_band_from_any_corner() {
        assert_eq!(Rect::from_corners((10.0, 50.0), (2.0, 20.0)), Rect::new(2, 20, 8, 30));
    }
}
