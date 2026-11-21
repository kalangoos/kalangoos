//! Ajustes › Sandbox (design "Ajustes · sandbox · modelos"): the Kalango VirtualDesk machines a sandbox is copied
//! from (kalangod `vm.listar`), how many copies are on now or kept to look at, the disk they take; on the right the
//! chosen template — its snapshot, disk, processors, memory and network, who uses it (agents with a sandbox, flows
//! with an "Em sandbox" block) — and the copies on now, each with "Parar" (`vm.desligar`). The machines themselves
//! are made and changed in VirtualDesk ("Abrir no VirtualDesk").

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A machine of VirtualDesk as `vm.listar` describes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub os: String,
    /// "rodando", "pausada", "desligada", "estado_salvo", "ocupada"
    pub state: String,
    pub cpus: u64,
    pub memory_mb: u64,
    pub disk_gb: f64,
    /// "nat" or "nenhuma"
    pub network: String,
    pub folders: usize,
    /// the snapshot a copy starts from, with memory (ready without booting); None: it boots cold
    pub memory_snapshot: Option<String>,
}

impl Machine {
    pub fn from_json(m: &Value) -> Option<Machine> {
        let snaps = m["snapshots"].as_array().cloned().unwrap_or_default();
        let current = m["snapshot_atual"].as_str();
        let with_mem: Vec<&Value> = snaps.iter().filter(|s| s["com_memoria"].as_bool().unwrap_or(false)).collect();
        // (the current snapshot when it has memory, else the newest one that does — as vm.clonar picks it)
        let snap = with_mem.iter().find(|s| s["tag"].as_str() == current).or(with_mem.last()).map(|s| s["nome"].as_str().or(s["tag"].as_str()).unwrap_or("").to_string());
        Some(Machine {
            id: m["id"].as_str()?.to_string(),
            name: m["nome"].as_str().unwrap_or("").to_string(),
            os: m["sistema"].as_str().unwrap_or("").to_string(),
            state: m["estado"].as_str().unwrap_or("").to_string(),
            cpus: m["cpus"].as_u64().unwrap_or(0),
            memory_mb: m["memoria_mb"].as_u64().unwrap_or(0),
            disk_gb: m["disco_gb"].as_f64().unwrap_or(0.0),
            network: m["rede"].as_str().unwrap_or("").to_string(),
            folders: m["pastas"].as_array().map(Vec::len).unwrap_or(0),
            memory_snapshot: snap,
        })
    }

    /// A sandbox's disposable copy (vm.clonar names them "sandbox-…"), not a template.
    pub fn is_copy(&self) -> bool {
        self.id.starts_with("sandbox-")
    }

    pub fn is_on(&self) -> bool {
        matches!(self.state.as_str(), "rodando" | "pausada" | "ocupada")
    }

    /// The name the screens give it.
    pub fn title(&self) -> String {
        if self.name.trim().is_empty() { self.id.clone() } else { self.name.clone() }
    }
}

/// The machines of a `vm.listar` answer.
pub fn machines(v: &Value) -> Vec<Machine> {
    v["maquinas"].as_array().or_else(|| v["valor"]["maquinas"].as_array()).into_iter().flatten().filter_map(Machine::from_json).collect()
}

/// The numbers on top: templates ready, copies on now, copies kept to look at, disk (GB) of them all.
pub fn stats(list: &[Machine]) -> (usize, usize, usize, f64) {
    let templates = list.iter().filter(|m| !m.is_copy()).count();
    let on = list.iter().filter(|m| m.is_copy() && m.is_on()).count();
    let kept = list.iter().filter(|m| m.is_copy() && !m.is_on()).count();
    (templates, on, kept, list.iter().map(|m| m.disk_gb).sum())
}

/// "2,1 GB", "18 GB".
pub fn gb_text(gb: f64) -> String {
    if gb < 10.0 { format!("{} GB", crate::fluxo_modelo::decimal(gb, 1)) } else { format!("{} GB", gb.round()) }
}

/// Who copies each template: agents with a sandbox (`config.sandbox.template`) and flows with an "Em sandbox" block.
pub fn users(agents: &[Value], flows: &[(String, Value)]) -> HashMap<String, Vec<String>> {
    let mut v: HashMap<String, Vec<String>> = HashMap::new();
    for a in agents {
        let a = if a["agent"].is_object() { &a["agent"] } else { a };
        if let Some(t) = a["config"]["sandbox"]["template"].as_str().filter(|t| !t.is_empty()) {
            v.entry(t.to_string()).or_default().push(a["name"].as_str().unwrap_or("").to_string());
        }
    }
    for (name, flow) in flows {
        for n in flow["nodes"].as_array().into_iter().flatten().filter(|n| n["type"] == json!("sandbox")) {
            if let Some(t) = n["data"]["template"].as_str().filter(|t| !t.is_empty()) {
                let e = v.entry(t.to_string()).or_default();
                if !e.contains(name) {
                    e.push(name.clone());
                }
            }
        }
    }
    v
}

/// A kalangod call off the main thread (vm.* go to VirtualDesk).
pub fn vm_call(tool: &str, args: Value, done: impl FnOnce(Result<Value, String>) + 'static) {
    let tool = tool.to_string();
    crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_chamar(&tool, args), done);
}

/// Opens Kalango VirtualDesk (where the machines are made and changed).
pub fn open_virtualdesk() {
    let bin = std::env::var("KALANGO_VIRTUALDESK").unwrap_or_else(|_| "kalango-virtualdesk".into());
    if let Err(e) = std::process::Command::new(&bin).spawn() {
        eprintln!("ajustes sandbox: VirtualDesk não abriu ({bin}): {e}");
    }
}

pub struct SandboxTab {
    pub body: gtk::Box,
    pub side: gtk::Box,
    numbers: [gtk::Label; 4],
    list: gtk::Box,
    empty: gtk::Label,
    machines: RefCell<Vec<Machine>>,
    users: RefCell<HashMap<String, Vec<String>>>,
    chosen: RefCell<String>,
    // the side
    s_name: gtk::Label,
    s_sub: gtk::Label,
    s_info: gtk::Box,
    s_defaults: gtk::Box,
    s_on: gtk::Box,
}

impl SandboxTab {
    pub fn new(side_width: i32) -> Rc<SandboxTab> {
        let body = caixa(Orientation::Vertical, 0, "");
        body.set_margin_bottom(16);
        let top = caixa(Orientation::Horizontal, 16, "");
        top.set_margin_top(16);
        let exp = rotulo(tr("Kalango VirtualDesk machine templates each sandbox is born from. Each run starts on a clean copy; the template never changes."), "central-sub");
        exp.set_xalign(0.0);
        exp.set_wrap(true);
        exp.set_max_width_chars(1);
        exp.set_hexpand(true);
        top.append(&exp);
        let create = gtk::Button::new();
        create.add_css_class("rotinas-nova");
        let cc = caixa(Orientation::Horizontal, 6, "");
        cc.append(&ui_icon("mais", "#1C1D24", 14));
        cc.append(&rotulo(tr("Create from a machine"), ""));
        create.set_child(Some(&cc));
        create.set_valign(Align::Center);
        create.set_tooltip_text(Some(tr("Opens VirtualDesk: prepare the machine there and it shows here as a template")));
        create.connect_clicked(|_| open_virtualdesk());
        top.append(&create);
        body.append(&top);
        let stats = caixa(Orientation::Horizontal, 12, "");
        stats.set_homogeneous(true);
        stats.set_margin_top(14);
        let stat = |t: &str, c: &str| {
            let b = caixa(Orientation::Vertical, 4, "linha-stat");
            let n = rotulo("0", "linha-stat-numero");
            if !c.is_empty() {
                n.add_css_class(c);
            }
            n.set_xalign(0.0);
            b.append(&n);
            let l = rotulo(t, "central-agente-sub");
            l.set_xalign(0.0);
            b.append(&l);
            stats.append(&b);
            n
        };
        let numbers = [stat(tr("templates ready"), "teal"), stat(tr("sandboxes on now"), "acento"), stat(tr("kept to look at"), "gold"), stat(tr("on disk"), "")];
        body.append(&stats);
        let list = caixa(Orientation::Vertical, 8, "");
        list.set_margin_top(14);
        let sc = gtk::ScrolledWindow::new();
        sc.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        sc.set_vexpand(true);
        sc.set_child(Some(&list));
        body.append(&sc);
        let empty = rotulo(tr("No VirtualDesk machine yet: make one in VirtualDesk and it becomes a template here."), "central-agente-sub");
        empty.set_xalign(0.0);
        empty.set_wrap(true);
        empty.set_visible(false);
        list.append(&empty);

        // the side: the chosen template
        let side = caixa(Orientation::Vertical, 0, "central-painel");
        side.set_size_request(side_width, -1);
        side.set_hexpand(false);
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("apps-pontos", "#A5A1E2", 14));
        cab.append(&rotulo(tr("CHOSEN TEMPLATE"), "central-painel-agente"));
        side.append(&cab);
        let head = caixa(Orientation::Horizontal, 12, "");
        head.set_margin_top(14);
        let ic = caixa(Orientation::Horizontal, 0, "ajustes-icone-grande");
        ic.set_size_request(44, 44);
        ic.set_hexpand(false);
        ic.set_valign(Align::Center);
        let g = ui_icon("apps-pontos", "#8E8F99", 16);
        g.set_hexpand(true);
        ic.append(&g);
        head.append(&ic);
        let hc = caixa(Orientation::Vertical, 2, "");
        hc.set_valign(Align::Center);
        hc.set_hexpand(true);
        let s_name = rotulo("", "linha-titulo");
        s_name.set_xalign(0.0);
        s_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        hc.append(&s_name);
        let s_sub = rotulo("", "central-agente-sub");
        s_sub.set_xalign(0.0);
        s_sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s_sub.set_max_width_chars(1);
        hc.append(&s_sub);
        head.append(&hc);
        side.append(&head);
        let s_info = caixa(Orientation::Vertical, 6, "central-cartao");
        s_info.add_css_class("memoria-info");
        s_info.set_margin_top(14);
        side.append(&s_info);
        let t2 = rotulo(tr("WHAT A COPY OF IT GETS"), "central-painel-secao");
        t2.set_xalign(0.0);
        t2.set_margin_top(18);
        side.append(&t2);
        let s_defaults = caixa(Orientation::Vertical, 6, "central-cartao");
        s_defaults.add_css_class("memoria-info");
        s_defaults.set_margin_top(10);
        side.append(&s_defaults);
        let t3 = rotulo(tr("ON NOW"), "central-painel-secao");
        t3.set_xalign(0.0);
        t3.set_margin_top(18);
        side.append(&t3);
        let s_on = caixa(Orientation::Vertical, 6, "central-cartao");
        s_on.add_css_class("memoria-info");
        s_on.set_margin_top(10);
        side.append(&s_on);
        let gap = caixa(Orientation::Vertical, 0, "");
        gap.set_vexpand(true);
        side.append(&gap);
        let open = gtk::Button::new();
        open.add_css_class("central-botao");
        let oc = caixa(Orientation::Horizontal, 8, "");
        oc.set_halign(Align::Center);
        oc.append(&ui_icon("apps-pontos", "#E2E2E6", 14));
        oc.append(&rotulo(tr("Open in VirtualDesk"), ""));
        open.set_child(Some(&oc));
        open.connect_clicked(|_| open_virtualdesk());
        side.append(&open);

        let t = Rc::new(SandboxTab {
            body, side, numbers, list, empty, machines: RefCell::default(), users: RefCell::default(), chosen: RefCell::default(), s_name, s_sub, s_info, s_defaults,
            s_on,
        });
        t.render();
        t
    }

    /// The machines (kalangod `vm.listar`) and who uses each template (agents, flows).
    pub fn reload(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        vm_call("vm.listar", json!({}), move |r| {
            let Some(t) = w.upgrade() else { return };
            match r {
                Ok(v) => *t.machines.borrow_mut() = machines(&v),
                Err(e) => {
                    t.machines.borrow_mut().clear();
                    t.empty.set_text(&format!("{} {e}", tr("VirtualDesk did not answer:")));
                }
            }
            t.render();
        });
        let w = Rc::downgrade(self);
        crate::fluxos::call("agents.list", json!({}), move |r| {
            let Some(t) = w.upgrade() else { return };
            let agents = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            let w2 = Rc::downgrade(&t);
            crate::fluxos::call("flows.list", json!({}), move |r| {
                let Some(t) = w2.upgrade() else { return };
                let list: Vec<(String, String)> = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default().iter().filter_map(|f| Some((f["id"].as_str()?.to_string(), f["name"].as_str().unwrap_or("").to_string()))).collect();
                let got: Rc<RefCell<Vec<(String, Value)>>> = Rc::default();
                let left = Rc::new(std::cell::Cell::new(list.len()));
                *t.users.borrow_mut() = users(&agents, &[]);
                t.render();
                for (id, name) in list {
                    let (w3, got, left, agents) = (Rc::downgrade(&t), got.clone(), left.clone(), agents.clone());
                    crate::fluxos::call("flows.get", json!({"id": id}), move |r| {
                        if let Ok(v) = r {
                            got.borrow_mut().push((name, v["flow"].clone()));
                        }
                        left.set(left.get() - 1);
                        if left.get() == 0 {
                            if let Some(t) = w3.upgrade() {
                                *t.users.borrow_mut() = users(&agents, &got.borrow());
                                t.render();
                            }
                        }
                    });
                }
            });
        });
    }

    fn render(self: &Rc<Self>) {
        let all = self.machines.borrow().clone();
        let (templates, on, kept, disk) = stats(&all);
        for (l, v) in self.numbers.iter().zip([templates.to_string(), on.to_string(), kept.to_string(), gb_text(disk)]) {
            l.set_text(&v);
        }
        while let Some(c) = self.list.last_child() {
            if c == self.empty {
                break;
            }
            self.list.remove(&c);
        }
        let tpl: Vec<Machine> = all.iter().filter(|m| !m.is_copy()).cloned().collect();
        self.empty.set_visible(tpl.is_empty());
        if !tpl.iter().any(|m| m.id == *self.chosen.borrow()) {
            *self.chosen.borrow_mut() = tpl.first().map(|m| m.id.clone()).unwrap_or_default();
        }
        let users = self.users.borrow().clone();
        for m in &tpl {
            self.list.append(&self.row(m, users.get(&m.id).cloned().unwrap_or_default()));
        }
        self.detail();
    }

    fn row(self: &Rc<Self>, m: &Machine, used: Vec<String>) -> gtk::Box {
        let card = caixa(Orientation::Horizontal, 14, "rotinas-item");
        card.add_css_class("ajustes-conector");
        if *self.chosen.borrow() == m.id {
            card.add_css_class("escolhido");
        }
        let ic = caixa(Orientation::Horizontal, 0, "rotinas-icone");
        ic.set_size_request(36, 36);
        ic.set_hexpand(false);
        ic.set_valign(Align::Center);
        let g = ui_icon("apps-pontos", "#8E8F99", 16);
        g.set_hexpand(true);
        ic.append(&g);
        card.append(&ic);
        let col = caixa(Orientation::Vertical, 4, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let l1 = caixa(Orientation::Horizontal, 8, "");
        l1.append(&rotulo(&m.title(), "rotinas-nome"));
        let chip = if m.memory_snapshot.is_some() { rotulo(tr("snapshot with memory"), "fx-chip") } else { rotulo(tr("disk only · boots cold"), "fx-chip") };
        chip.add_css_class(if m.memory_snapshot.is_some() { "teal" } else { "ouro" });
        chip.set_valign(Align::Center);
        l1.append(&chip);
        if m.is_on() {
            let c = rotulo(tr("on"), "fx-chip");
            c.add_css_class("azul");
            c.set_valign(Align::Center);
            l1.append(&c);
        }
        col.append(&l1);
        let who = if used.is_empty() { tr("nobody").to_string() } else { used.join(", ") };
        let sub = format!("{} · {}   ·   {} {who}", gb_text(m.disk_gb), if m.os.is_empty() { &m.id } else { &m.os }, tr("used by:"));
        let s = rotulo(&sub, "central-agente-sub");
        s.set_xalign(0.0);
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s.set_max_width_chars(1);
        s.set_hexpand(true);
        col.append(&s);
        card.append(&col);
        let click = gtk::GestureClick::new();
        let (w, id) = (Rc::downgrade(self), m.id.clone());
        click.connect_released(move |_, _, _, _| {
            if let Some(t) = w.upgrade() {
                *t.chosen.borrow_mut() = id.clone();
                t.render();
            }
        });
        card.add_controller(click);
        card
    }

    fn detail(self: &Rc<Self>) {
        for c in [&self.s_info, &self.s_defaults, &self.s_on] {
            while let Some(x) = c.first_child() {
                c.remove(&x);
            }
        }
        let all = self.machines.borrow().clone();
        let chosen = self.chosen.borrow().clone();
        let info = |b: &gtk::Box, name: &str, value: &str, class: &str| {
            let l = caixa(Orientation::Horizontal, 8, "");
            let n = rotulo(name, "central-agente-sub");
            n.set_hexpand(true);
            n.set_xalign(0.0);
            l.append(&n);
            let v = rotulo(value, "memoria-info-valor");
            if !class.is_empty() {
                v.add_css_class(class);
            }
            v.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            v.set_max_width_chars(28);
            l.append(&v);
            b.append(&l);
        };
        match all.iter().find(|m| m.id == chosen) {
            Some(m) => {
                self.s_name.set_text(&m.title());
                self.s_sub.set_text(&trf("from VirtualDesk's machine \"{id}\"", &[("id", m.id.clone())]));
                match &m.memory_snapshot {
                    Some(s) => info(&self.s_info, tr("Snapshot"), &format!("{s} · {}", tr("with memory")), ""),
                    None => info(&self.s_info, tr("Snapshot"), tr("none: each copy boots cold"), "ouro"),
                }
                info(&self.s_info, tr("Disk"), &format!("{} + {}", gb_text(m.disk_gb), tr("a copy per sandbox")), "");
                info(&self.s_info, tr("System"), if m.os.is_empty() { "—" } else { &m.os }, "");
                info(&self.s_info, tr("State"), &state_word(&m.state), if m.is_on() { "teal" } else { "" });
                info(&self.s_defaults, tr("Processors and memory"), &format!("{} · {}", m.cpus, gb_text(m.memory_mb as f64 / 1024.0)), "");
                info(&self.s_defaults, tr("Network"), tr("chosen by who uses it"), "");
                info(&self.s_defaults, tr("Shared folders"), &if m.folders == 0 { tr("none").to_string() } else { trf("{n} of the template's", &[("n", m.folders.to_string())]) }, "");
                info(&self.s_defaults, tr("Used by"), &self.users.borrow().get(&m.id).map(|u| u.join(", ")).unwrap_or_else(|| tr("nobody").to_string()), "");
            }
            None => {
                self.s_name.set_text(tr("No template"));
                self.s_sub.set_text(tr("make a machine in VirtualDesk"));
            }
        }
        // the copies on now (and the ones kept off, to look at)
        let copies: Vec<Machine> = all.iter().filter(|m| m.is_copy()).cloned().collect();
        if copies.is_empty() {
            let l = rotulo(tr("No sandbox on now."), "central-agente-sub");
            l.set_xalign(0.0);
            self.s_on.append(&l);
        }
        for c in copies {
            let row = caixa(Orientation::Horizontal, 8, "");
            let dot = caixa(Orientation::Vertical, 0, "ajustes-ponto");
            if c.is_on() {
                dot.add_css_class("ok");
            }
            dot.set_valign(Align::Center);
            row.append(&dot);
            row.append(&rotulo(&c.id, "memoria-info-valor"));
            let st = rotulo(&state_word(&c.state), "central-agente-sub");
            st.set_hexpand(true);
            st.set_xalign(0.0);
            row.append(&st);
            let stop = gtk::Button::with_label(if c.is_on() { tr("Stop") } else { tr("Delete") });
            stop.add_css_class("rotinas-texto");
            stop.add_css_class("perigo");
            let (w, id, on) = (Rc::downgrade(self), c.id.clone(), c.is_on());
            stop.connect_clicked(move |b| {
                b.set_sensitive(false);
                let w = w.clone();
                let (tool, args) = if on { ("vm.desligar", json!({"id": id, "forcar": true})) } else { ("vm.apagar", json!({"id": id})) };
                vm_call(tool, args, move |r| {
                    if let Err(e) = r {
                        eprintln!("ajustes sandbox: {e}");
                    }
                    if let Some(t) = w.upgrade() {
                        t.reload();
                    }
                });
            });
            row.append(&stop);
            self.s_on.append(&row);
        }
    }
}

/// A machine's state in words.
fn state_word(s: &str) -> String {
    match s {
        "rodando" => tr("on").to_string(),
        "pausada" => tr("paused").to_string(),
        "estado_salvo" => tr("saved, off").to_string(),
        "ocupada" => tr("busy").to_string(),
        _ => tr("off").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_copies_and_their_numbers() {
        let v = json!({"maquinas": [
            {"id": "modelo-alpine", "nome": "modelo-alpine", "estado": "desligada", "cpus": 2, "memoria_mb": 1024, "disco_gb": 2.0,
             "snapshots": [{"tag": "s1", "nome": "base limpa", "com_memoria": true}], "snapshot_atual": "s1"},
            {"id": "sandbox-1a2b", "nome": "", "estado": "rodando", "disco_gb": 0.5},
            {"id": "sandbox-3c4d", "estado": "desligada", "disco_gb": 0.5},
            {"id": "win", "nome": "Windows", "estado": "desligada", "disco_gb": 18.0, "snapshots": [{"tag": "x", "com_memoria": false}]}]});
        let list = machines(&v);
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].memory_snapshot.as_deref(), Some("base limpa"));
        assert_eq!(list[3].memory_snapshot, None);
        assert!(list[1].is_copy() && list[1].is_on());
        assert_eq!(stats(&list), (2, 1, 1, 21.0));
        assert_eq!(list[1].title(), "sandbox-1a2b");
    }

    #[test]
    fn who_uses_a_template() {
        let agents = vec![json!({"agent": {"name": "Analisador", "config": {"sandbox": {"template": "t"}}}}), json!({"agent": {"name": "Outro", "config": {}}})];
        let flows = vec![("Anexos".to_string(), json!({"nodes": [{"type": "sandbox", "data": {"template": "t"}}, {"type": "sandbox", "data": {"template": "t"}}]}))];
        assert_eq!(users(&agents, &flows).get("t").cloned(), Some(vec!["Analisador".to_string(), "Anexos".to_string()]));
    }
}
