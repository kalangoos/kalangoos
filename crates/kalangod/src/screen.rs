//! An agent operating the screen (the `tela.*` tools): the rules, as pure functions over what niri reports.
//!
//! Every action is decided here before the screen helper (kalango-tela) touches anything:
//! - the screen is locked, or an approval, a password or a permission prompt is on screen → refused (an agent never
//!   answers its own approvals, never types a password);
//! - the target is in a window no agent may use (Settings › Privacy and security, Users, Agents and AI; the Agents
//!   center; terminals, which agents reach through the command tools and their own checks) → refused;
//! - the target is outside the window the person approved → refused;
//! - typing into a password field (AT-SPI's password role) → refused.
//!
//! Coordinates: the model sees a picture of a region of an output (the approved window, or the whole output) scaled
//! down; `ImageMap` turns its pixels into the output's logical coordinates and back.

use kalango_i18n::{tr, trf};
use serde_json::{json, Value};

/// A rectangle in an output's logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// A picture of `region` (logical) at `img_w × img_h` pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageMap {
    pub region: Rect,
    pub img_w: u32,
    pub img_h: u32,
}

impl ImageMap {
    /// A pixel of the picture → the output's logical point (its centre).
    pub fn to_output(self, x: f64, y: f64) -> Option<(f64, f64)> {
        if self.img_w == 0 || self.img_h == 0 || x < 0.0 || y < 0.0 || x > self.img_w as f64 || y > self.img_h as f64 {
            return None;
        }
        Some((self.region.x + x * self.region.w / self.img_w as f64, self.region.y + y * self.region.h / self.img_h as f64))
    }

    /// The output's logical point → the picture's pixel.
    pub fn to_image(self, x: f64, y: f64) -> (i64, i64) {
        let k = |v: f64, o: f64, size: f64, img: u32| (((v - o) * img as f64 / size).round() as i64).clamp(0, img as i64);
        (k(x, self.region.x, self.region.w, self.img_w), k(y, self.region.y, self.region.h, self.img_h))
    }

    /// The region in the output's physical pixels (what the capture crops).
    pub fn physical(self, scale: f64) -> Value {
        json!({"x": (self.region.x * scale).round(), "y": (self.region.y * scale).round(), "w": (self.region.w * scale).round(), "h": (self.region.h * scale).round()})
    }
}

/// An output as niri reports it (logical size and scale).
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

/// A window on screen: its rectangle on its output (only windows of the active workspaces are on screen).
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub pid: i32,
    pub output: String,
    pub rect: Rect,
    pub focused: bool,
    pub floating: bool,
}

/// What the screen holds now (from niri's IPC, through the helper).
#[derive(Debug, Clone, Default)]
pub struct Screen {
    pub outputs: Vec<Output>,
    pub windows: Vec<Window>,
    /// layer surfaces' namespaces (the shell's and the system's overlays)
    pub layers: Vec<String>,
}

/// niri's answers → `Screen`. Windows without a position (patch 0004 gives the tiled ones) are left out.
pub fn parse_screen(state: &Value) -> Screen {
    let mut s = Screen::default();
    for o in state["saidas"].as_object().map(|m| m.values().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        let l = &o["logical"];
        if l.is_null() {
            continue;
        }
        s.outputs.push(Output {
            name: o["name"].as_str().unwrap_or("").into(),
            x: l["x"].as_f64().unwrap_or(0.0),
            y: l["y"].as_f64().unwrap_or(0.0),
            w: l["width"].as_f64().unwrap_or(0.0),
            h: l["height"].as_f64().unwrap_or(0.0),
            scale: l["scale"].as_f64().unwrap_or(1.0),
        });
    }
    s.outputs.sort_by(|a, b| a.name.cmp(&b.name));
    let spaces = state["espacos"].as_array().cloned().unwrap_or_default();
    let active: Vec<(u64, String)> = spaces.iter().filter(|w| w["is_active"] == true)
        .map(|w| (w["id"].as_u64().unwrap_or(0), w["output"].as_str().unwrap_or("").to_string())).collect();
    for w in state["janelas"].as_array().cloned().unwrap_or_default() {
        let Some((_, output)) = active.iter().find(|(id, _)| Some(*id) == w["workspace_id"].as_u64()) else { continue };
        if w["is_minimized"] == true {
            continue;
        }
        let lay = &w["layout"];
        let Some(pos) = lay["tile_pos_in_workspace_view"].as_array() else { continue };
        let off = lay["window_offset_in_tile"].as_array();
        let size = lay["window_size"].as_array();
        let f = |a: Option<&Vec<Value>>, i: usize| a.and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(0.0);
        s.windows.push(Window {
            id: w["id"].as_u64().unwrap_or(0),
            title: w["title"].as_str().unwrap_or("").into(),
            app_id: w["app_id"].as_str().unwrap_or("").into(),
            pid: w["pid"].as_i64().unwrap_or(0) as i32,
            output: output.clone(),
            rect: Rect { x: f(Some(pos), 0) + f(off, 0), y: f(Some(pos), 1) + f(off, 1), w: f(size, 0), h: f(size, 1) },
            focused: w["is_focused"] == true,
            floating: w["is_floating"] == true,
        });
    }
    s.layers = state["camadas"].as_array().cloned().unwrap_or_default().iter().filter_map(|l| l["namespace"].as_str().map(str::to_string)).collect();
    s
}

impl Screen {
    pub fn output(&self, name: &str) -> Option<&Output> {
        self.outputs.iter().find(|o| o.name == name)
    }

    pub fn window(&self, id: u64) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn focused(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.focused)
    }

    /// The window under a point of an output: floating ones are on top, then the focused one.
    pub fn window_at(&self, output: &str, x: f64, y: f64) -> Option<&Window> {
        let mut under: Vec<&Window> = self.windows.iter().filter(|w| w.output == output && w.rect.contains(x, y)).collect();
        under.sort_by_key(|w| (!w.floating, !w.focused));
        under.first().copied()
    }

    /// The output an action without a window goes to: the focused window's, else the first.
    pub fn main_output(&self) -> Option<&Output> {
        self.focused().and_then(|w| self.output(&w.output)).or(self.outputs.first())
    }
}

/// What the person approved: one window, or the whole screen (one output).
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    Screen { output: String },
    Window { id: u64, title: String, app_id: String },
}

/// Overlays no agent acts while they show (layer namespaces): why.
pub fn protected_layer(namespace: &str) -> Option<String> {
    Some(match namespace {
        "kalango-aprovacao" | "kalango-permissao" => tr("an approval is waiting for the person on screen: an agent never answers approvals").into(),
        "kalango-autenticacao" => tr("a password prompt is on screen: an agent never types passwords").into(),
        "kalango-wifi-senha" => tr("a Wi-Fi password prompt is on screen: an agent never types passwords").into(),
        "kalango-entrada" | "kalango-travado" => tr("the login or lock screen is on: an agent never acts there").into(),
        _ => return None,
    })
}

const TERMINALS: &[&str] = &["os.kalango.Terminal", "org.gnome.Terminal", "org.gnome.Console", "kitty", "foot", "footclient", "Alacritty", "alacritty",
    "org.wezfurlong.wezterm", "com.mitchellh.ghostty", "xterm", "org.kde.konsole", "com.raggesilver.BlackBox", "terminator", "tilix", "com.gexperts.Tilix"];

/// The settings pages and system windows an agent never touches, by the window's title ("Privacy and security —
/// Settings", in English or in the person's language): what they change is the agents' own leash.
pub fn protected_window(w: &Window) -> Option<String> {
    let app = w.app_id.as_str();
    if TERMINALS.contains(&app) || app.to_lowercase().contains("terminal") {
        return Some(tr("terminals are reached through the system's command tools, never by typing on the screen").into());
    }
    if app == "polkit-gnome-authentication-agent-1" || app.contains("polkit") || app == "org.kde.polkit-kde-authentication-agent-1" || app == "kalango-aprovacao" {
        return Some(tr("a password or approval window: an agent never answers it").into());
    }
    let t = w.title.as_str();
    let has = |en: &'static str| t.contains(en) || t.contains(tr(en));
    let settings = has("Settings");
    if settings && (has("Privacy and security") || has("Users") || has("Agents and AI")) {
        return Some(tr("Settings › Privacy and security, Users and Agents and AI are never an agent's to change").into());
    }
    // (the agents center: permissions, memory, the timeline — the agents' own rules)
    if t == "Agents" || t == tr("Agents") || app == "os.kalango.Agentes" {
        return Some(tr("the agents center (permissions, memory, timeline) is never an agent's to change").into());
    }
    None
}

/// What an action aims at.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// a point of an output (logical)
    Point { output: String, x: f64, y: f64 },
    /// the keyboard: the focused window
    Keys,
}

/// The facts besides the screen: the session is locked, approvals wait for this person.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub locked: bool,
    pub pending_approvals: usize,
    /// the banner's Stop button on the output (the shell tells where it is): the person's, never an agent's click
    pub stop_button: Option<(String, Rect)>,
}

/// Why this action is refused (None: it may go ahead). `scope`: None while asking to start a session.
pub fn refusal(screen: &Screen, facts: &Facts, scope: Option<&Scope>, target: &Target) -> Option<String> {
    if facts.locked {
        return Some(tr("the screen is locked: an agent never acts on the lock screen").into());
    }
    if facts.pending_approvals > 0 {
        return Some(tr("an approval is waiting for the person: an agent never answers approvals, not even by clicking").into());
    }
    if let Some(why) = screen.layers.iter().find_map(|l| protected_layer(l)) {
        return Some(why);
    }
    let window = match target {
        Target::Point { output, x, y } => {
            if screen.output(output).is_none_or(|o| *x < 0.0 || *y < 0.0 || *x >= o.w || *y >= o.h) {
                return Some(tr("that point is outside the screen").into());
            }
            // (with a few pixels around it: a click on its edge would still press it)
            if facts.stop_button.as_ref().is_some_and(|(o, r)| o == output && Rect { x: r.x - 4.0, y: r.y - 4.0, w: r.w + 8.0, h: r.h + 8.0 }.contains(*x, *y)) {
                return Some(tr("that is the banner's Stop button, the person's: an agent never presses it").into());
            }
            screen.window_at(output, *x, *y)
        }
        Target::Keys => screen.focused(),
    };
    if let Some(w) = window {
        if let Some(why) = protected_window(w) {
            return Some(why);
        }
    }
    match (scope, target) {
        (Some(Scope::Window { id, title, .. }), _) => match window {
            Some(w) if w.id == *id => None,
            Some(w) => Some(trf("the person allowed only the window “{allowed}”; this is “{other}”", &[("allowed", title.clone()), ("other", w.title.clone())])),
            None => Some(trf("the person allowed only the window “{allowed}”; that point is outside it", &[("allowed", title.clone())])),
        },
        (Some(Scope::Screen { output }), Target::Point { output: o, .. }) if o != output => Some(tr("the person allowed another screen").into()),
        (_, Target::Keys) if window.is_none() => Some(tr("no window has the keyboard: click the field to type in first").into()),
        _ => None,
    }
}

/// Typing goes into the focused element: never a password field.
pub fn typing_refusal(elements: &[Value]) -> Option<String> {
    elements.iter().any(|e| e["foco"] == true && e["senha"] == true)
        .then(|| tr("the field with the keyboard is a password field: an agent never types passwords").into())
}

/// The elements as the model sees them: ids "e1"…, centres and boxes in the picture's pixels. `origin`: the
/// window's position on its output (the helper gives boxes relative to the window).
pub fn elements_for_model(elements: &[Value], origin: (f64, f64), map: &ImageMap) -> (Vec<Value>, Vec<(String, f64, f64)>) {
    let mut shown = Vec::new();
    let mut centres = Vec::new();
    for (i, e) in elements.iter().enumerate() {
        let id = format!("e{}", i + 1);
        let (x, y, w, h) = (e["x"].as_f64().unwrap_or(0.0) + origin.0, e["y"].as_f64().unwrap_or(0.0) + origin.1, e["w"].as_f64().unwrap_or(0.0), e["h"].as_f64().unwrap_or(0.0));
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        // (only what is inside the picture)
        if !map.region.contains(cx, cy) {
            continue;
        }
        let (ix, iy) = map.to_image(cx, cy);
        let (bx, by) = map.to_image(x, y);
        let (bx2, by2) = map.to_image(x + w, y + h);
        let mut v = json!({"id": id, "papel": e["papel"], "nome": e["nome"], "x": ix, "y": iy, "caixa": [bx, by, bx2 - bx, by2 - by]});
        for k in ["foco", "editavel", "senha", "marcado", "valor"] {
            if e.get(k).is_some_and(|v| v != false && !v.is_null()) {
                v[k] = e[k].clone();
            }
        }
        if e["clicavel"] == false {
            v["clicavel"] = json!(false);
        }
        centres.push((id, cx, cy));
        shown.push(v);
    }
    (shown, centres)
}

/// An element by its id, or by its name when the model wrote the label ("Salvar") instead of the id.
pub fn find_element<'a>(centres: &'a [(String, f64, f64)], names: &[(String, String)], wanted: &str) -> Option<&'a (String, f64, f64)> {
    let w = wanted.trim();
    if let Some(c) = centres.iter().find(|c| c.0.eq_ignore_ascii_case(w)) {
        return Some(c);
    }
    let lw = w.trim_matches(|c| c == '"' || c == '“' || c == '”').to_lowercase();
    let id = names.iter().find(|(_, n)| n.to_lowercase() == lw).or_else(|| names.iter().find(|(_, n)| !lw.is_empty() && n.to_lowercase().contains(&lw)))?;
    centres.iter().find(|c| c.0 == id.0)
}

/// How long to wait before the next action (the rate limit): `min_ms` between actions.
pub fn wait_before(last_ms_ago: Option<u64>, min_ms: u64) -> u64 {
    last_ms_ago.map(|ago| min_ms.saturating_sub(ago)).unwrap_or(0)
}

/// The names that appeared and disappeared between two element lists (what an action changed).
pub fn changes(before: &[Value], after: &[Value]) -> Value {
    let names = |l: &[Value]| l.iter().filter_map(|e| e["nome"].as_str().filter(|n| !n.is_empty()).map(|n| format!("{} \"{}\"", e["papel"].as_str().unwrap_or(""), n))).collect::<Vec<_>>();
    let (b, a) = (names(before), names(after));
    let novos: Vec<&String> = a.iter().filter(|n| !b.contains(n)).take(15).collect();
    let sumiram: Vec<&String> = b.iter().filter(|n| !a.contains(n)).take(15).collect();
    json!({"apareceram": novos, "sumiram": sumiram})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Screen {
        let state = json!({
            "saidas": {"winit": {"name": "winit", "logical": {"x": 0, "y": 0, "width": 1280, "height": 800, "scale": 1.0}}},
            "espacos": [{"id": 1, "output": "winit", "is_active": true}, {"id": 2, "output": "winit", "is_active": false}],
            "janelas": [
                {"id": 10, "title": "notas.txt — Text Editor", "app_id": "os.kalango.Editor", "pid": 50, "workspace_id": 1, "is_focused": true, "is_floating": false,
                 "layout": {"tile_pos_in_workspace_view": [100.0, 50.0], "window_offset_in_tile": [0.0, 0.0], "window_size": [800, 600]}},
                {"id": 11, "title": "Privacy and security — Settings", "app_id": "os.kalango.Shell", "pid": 40, "workspace_id": 1, "is_focused": false, "is_floating": true,
                 "layout": {"tile_pos_in_workspace_view": [700.0, 100.0], "window_offset_in_tile": [0.0, 0.0], "window_size": [400, 300]}},
                {"id": 12, "title": "Terminal", "app_id": "os.kalango.Terminal", "pid": 41, "workspace_id": 2, "is_focused": false, "is_floating": false,
                 "layout": {"tile_pos_in_workspace_view": [0.0, 0.0], "window_offset_in_tile": [0.0, 0.0], "window_size": [800, 600]}},
            ],
            "camadas": [{"namespace": "kalango-barra"}],
        });
        parse_screen(&state)
    }

    #[test]
    fn coordinates_map_both_ways() {
        // the window 800×600 at (100, 50), seen as 400×300
        let m = ImageMap { region: Rect { x: 100.0, y: 50.0, w: 800.0, h: 600.0 }, img_w: 400, img_h: 300 };
        assert_eq!(m.to_output(200.0, 150.0), Some((500.0, 350.0)));
        assert_eq!(m.to_image(500.0, 350.0), (200, 150));
        assert_eq!(m.to_output(0.0, 0.0), Some((100.0, 50.0)));
        assert_eq!(m.to_output(401.0, 10.0), None, "outside the picture");
        // a scale of 2: the physical crop is twice the logical
        assert_eq!(m.physical(2.0), json!({"x": 200.0, "y": 100.0, "w": 1600.0, "h": 1200.0}));
    }

    #[test]
    fn the_screen_from_niri() {
        let s = screen();
        assert_eq!(s.windows.len(), 2, "the window of another workspace is not on screen");
        assert_eq!(s.window_at("winit", 750.0, 150.0).unwrap().id, 11, "the floating window is on top");
        assert_eq!(s.window_at("winit", 150.0, 100.0).unwrap().id, 10);
        assert!(s.window_at("winit", 50.0, 10.0).is_none());
        assert_eq!(s.focused().unwrap().id, 10);
    }

    #[test]
    fn refusals() {
        let s = screen();
        let scope = Scope::Window { id: 10, title: "notas.txt — Text Editor".into(), app_id: "os.kalango.Editor".into() };
        let ok = Facts::default();
        let at = |x, y| Target::Point { output: "winit".into(), x, y };
        assert_eq!(refusal(&s, &ok, Some(&scope), &at(150.0, 100.0)), None);
        assert_eq!(refusal(&s, &ok, Some(&scope), &Target::Keys), None);
        assert_eq!(refusal(&s, &ok, Some(&scope), &at(50.0, 10.0)), Some(trf("the person allowed only the window “{allowed}”; that point is outside it", &[("allowed", "notas.txt — Text Editor".into())])), "outside the allowed window");
        assert!(refusal(&s, &ok, Some(&scope), &at(2000.0, 10.0)).is_some(), "off the screen");
        // Settings › Privacy: never, whatever the scope
        assert!(refusal(&s, &ok, None, &at(750.0, 150.0)).is_some());
        assert!(refusal(&s, &ok, Some(&Scope::Screen { output: "winit".into() }), &at(750.0, 150.0)).is_some());
        // locked, an approval waiting, an approval or password overlay on screen
        assert_eq!(refusal(&s, &Facts { locked: true, ..ok.clone() }, Some(&scope), &at(150.0, 100.0)).as_deref(), Some(tr("the screen is locked: an agent never acts on the lock screen")));
        assert!(refusal(&s, &Facts { pending_approvals: 1, ..ok.clone() }, Some(&scope), &at(150.0, 100.0)).is_some());
        for ns in ["kalango-aprovacao", "kalango-autenticacao", "kalango-wifi-senha", "kalango-entrada"] {
            let mut s2 = s.clone();
            s2.layers.push(ns.into());
            assert!(refusal(&s2, &ok, Some(&scope), &at(150.0, 100.0)).is_some(), "{ns}");
            assert!(refusal(&s2, &ok, Some(&scope), &Target::Keys).is_some(), "{ns}");
        }
    }

    #[test]
    fn the_banners_stop_button_is_never_an_agents() {
        let s = screen();
        let scope = Scope::Window { id: 10, title: "notas.txt — Text Editor".into(), app_id: "os.kalango.Editor".into() };
        // the banner over the Editor's top: its Stop button at 600,74 (90×25)
        let f = Facts { stop_button: Some(("winit".into(), Rect { x: 600.0, y: 74.0, w: 90.0, h: 25.0 })), ..Default::default() };
        let at = |x, y| Target::Point { output: "winit".into(), x, y };
        assert_eq!(refusal(&s, &f, Some(&scope), &at(640.0, 86.0)), Some(tr("that is the banner's Stop button, the person's: an agent never presses it").into()));
        assert!(refusal(&s, &f, Some(&scope), &at(598.0, 72.0)).is_some(), "its edge too");
        assert_eq!(refusal(&s, &f, Some(&scope), &at(560.0, 86.0)), None, "the rest of the banner lets the click through to the window");
        assert_eq!(refusal(&s, &f, Some(&scope), &Target::Keys), None);
        let other = Facts { stop_button: Some(("HDMI-1".into(), Rect { x: 600.0, y: 74.0, w: 90.0, h: 25.0 })), ..Default::default() };
        assert_eq!(refusal(&s, &other, Some(&scope), &at(640.0, 86.0)), None, "another output's banner");
    }

    #[test]
    fn protected_windows() {
        let w = |title: &str, app: &str| Window { id: 1, title: title.into(), app_id: app.into(), pid: 1, output: "o".into(), rect: Rect::default(), focused: true, floating: false };
        assert!(protected_window(&w("Users — Settings", "os.kalango.Shell")).is_some());
        assert!(protected_window(&w("Agents and AI — Settings", "os.kalango.Shell")).is_some());
        assert!(protected_window(&w("Display — Settings", "os.kalango.Shell")).is_none());
        assert!(protected_window(&w("~ — Terminal", "os.kalango.Terminal")).is_some());
        assert!(protected_window(&w("bash", "kitty")).is_some());
        assert!(protected_window(&w("Agents", "os.kalango.Shell")).is_some());
        assert!(protected_window(&w("Calculator", "os.kalango.Calculadora")).is_none());
    }

    #[test]
    fn password_fields_are_not_typed_into() {
        assert!(typing_refusal(&[json!({"papel": "password field", "foco": true, "senha": true})]).is_some());
        assert!(typing_refusal(&[json!({"papel": "password field", "foco": false, "senha": true}), json!({"papel": "text field", "foco": true, "senha": false})]).is_none());
    }

    #[test]
    fn elements_by_id_or_name() {
        let m = ImageMap { region: Rect { x: 100.0, y: 50.0, w: 800.0, h: 600.0 }, img_w: 400, img_h: 300 };
        let els = vec![
            json!({"papel": "button", "nome": "Salvar", "x": 700, "y": 10, "w": 80, "h": 30, "clicavel": true}),
            json!({"papel": "text field", "nome": "", "x": 0, "y": 60, "w": 800, "h": 500, "clicavel": true, "foco": true, "editavel": true, "valor": "ola"}),
            json!({"papel": "button", "nome": "Fora", "x": 900, "y": 10, "w": 80, "h": 30, "clicavel": true}),
        ];
        let (shown, centres) = elements_for_model(&els, (100.0, 50.0), &m);
        assert_eq!(shown.len(), 2, "the one outside the picture is not shown");
        assert_eq!(shown[0]["id"], "e1");
        assert_eq!(shown[0]["x"], 370);
        assert_eq!(shown[0]["y"], 13);
        assert_eq!(shown[1]["valor"], "ola");
        assert_eq!(centres[0], ("e1".to_string(), 840.0, 75.0));
        let names = vec![("e1".to_string(), "Salvar".to_string())];
        assert_eq!(find_element(&centres, &names, "E1").unwrap().0, "e1");
        assert_eq!(find_element(&centres, &names, "“Salvar”").unwrap().0, "e1");
        assert_eq!(find_element(&centres, &names, "salv").unwrap().0, "e1");
        assert!(find_element(&centres, &names, "e9").is_none());
    }

    #[test]
    fn rate_limit_and_changes() {
        assert_eq!(wait_before(None, 150), 0);
        assert_eq!(wait_before(Some(50), 150), 100);
        assert_eq!(wait_before(Some(500), 150), 0);
        let c = changes(&[json!({"papel": "button", "nome": "Salvar"})], &[json!({"papel": "label", "nome": "Salvo"})]);
        assert_eq!(c["apareceram"], json!(["label \"Salvo\""]));
        assert_eq!(c["sumiram"], json!(["button \"Salvar\""]));
    }
}
