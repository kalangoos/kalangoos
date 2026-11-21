//! The agent editor's "Onde roda" (design "Agente · sandbox (VirtualDesk)"): an assistant runs on this computer, or
//! each run starts on a clean copy of a VirtualDesk template (`config.sandbox`: template, network, on_end, inputs,
//! outputs — kalango-fluxos `sandbox::Settings`). The section sits at the end of "Avançado"; beside it, while the
//! sandbox is on, "Como fica": what crosses between this computer and the machine, and what the agent may do there.

use crate::ajustes_sandbox::{gb_text, machines, vm_call, Machine};
use crate::fluxo_modelo as m;
use crate::fluxos_ui::{self as ui, Segmented};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{n_, tr};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The network choices, as the dropdown says them.
const NETWORKS: [(&str, &str); 3] = [("nenhuma", n_("No network")), ("internet", n_("Internet only (not the home network)")), ("casa", n_("Home network"))];
/// The machine's time limits offered (seconds; None: no limit): the engine's `time_limit_s`, 1 s to a day.
const TIME_LIMITS: [Option<u64>; 7] = [None, Some(60), Some(300), Some(600), Some(1800), Some(3600), Some(7200)];

/// A time limit as the dropdown says it ("10 min", "1 h", "sem limite").
pub fn time_limit_text(t: Option<u64>) -> String {
    match t {
        None => tr("no limit").to_string(),
        Some(s) if s >= 3600 && s % 3600 == 0 => format!("{} h", s / 3600),
        Some(s) if s >= 60 && s % 60 == 0 => format!("{} min", s / 60),
        Some(s) => format!("{s} s"),
    }
}

/// The dropdown's choices: the usual ones, and the saved limit when it is none of them.
pub fn time_limit_options(saved: Option<u64>) -> Vec<Option<u64>> {
    let mut v: Vec<Option<u64>> = TIME_LIMITS.to_vec();
    if saved.is_some() && !v.contains(&saved) {
        v.push(saved);
    }
    v
}

/// What happens to the copy at the end.
const ENDS: [(&str, &str); 3] = [("apagar", n_("Delete the machine")), ("guardar", n_("Keep it, turned off")), ("se_falhar", n_("Keep it if something failed"))];

/// A sandbox with nothing chosen yet (the person picks the template).
pub fn new_settings(template: &str) -> Value {
    json!({"template": template, "network": "nenhuma", "on_end": "apagar", "inputs": [], "outputs": []})
}

/// How many files a way back brings: a folder's "/*" is every file in it.
pub fn output_reach(from: &str) -> String {
    if from.trim_end().ends_with("/*") { tr("every file of the folder").to_string() } else { tr("1 file").to_string() }
}

/// The folders of this computer the inputs come from and the outputs go to ("~/Documentos · ~/Downloads").
pub fn host_folders(s: &Value) -> String {
    let mut v: Vec<String> = Vec::new();
    for (list, key) in [("inputs", "from"), ("outputs", "to")] {
        for c in s[list].as_array().into_iter().flatten() {
            let p = c[key].as_str().unwrap_or("").trim();
            if p.starts_with('{') || p.is_empty() {
                continue;
            }
            // (the folder: two levels at most, "~/Documentos")
            let parts: Vec<&str> = p.trim_end_matches('/').split('/').collect();
            let f = parts.iter().take(2).copied().collect::<Vec<_>>().join("/");
            if !f.is_empty() && !v.contains(&f) {
                v.push(f);
            }
        }
    }
    v.join(" · ")
}

/// A path in a table: shown cut in the middle (the file's name stays in view) with the whole path in a tooltip; a
/// click edits it in `entry`, and leaving the box shows it cut again.
fn path_cell(entry: &gtk::Entry) -> gtk::Stack {
    let st = gtk::Stack::new();
    st.set_hexpand(true);
    st.set_hhomogeneous(true);
    let show = gtk::Button::new();
    show.add_css_class("fx-caminho");
    let l = rotulo("", "fx-caminho-texto");
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    l.set_width_chars(1);
    show.set_child(Some(&l));
    st.add_named(&show, Some("ver"));
    st.add_named(entry, Some("editar"));
    let sync = {
        let (l, show, e) = (l.clone(), show.clone(), entry.clone());
        move || {
            let t = e.text().to_string();
            let shown = if t.is_empty() { e.placeholder_text().map(|p| p.to_string()).unwrap_or_default() } else { t.clone() };
            l.set_text(&shown);
            if t.is_empty() { l.add_css_class("vazio") } else { l.remove_css_class("vazio") }
            show.set_tooltip_text(Some(&t).filter(|t| !t.is_empty()).map(String::as_str));
        }
    };
    sync();
    // (an empty line starts in its box: there is nothing to show yet)
    st.set_visible_child_name(if entry.text().is_empty() { "editar" } else { "ver" });
    let (st2, e) = (st.clone(), entry.clone());
    show.connect_clicked(move |_| {
        st2.set_visible_child_name("editar");
        e.grab_focus();
    });
    let focus = gtk::EventControllerFocus::new();
    let (st2, e, s2) = (st.clone(), entry.clone(), sync.clone());
    focus.connect_leave(move |_| {
        s2();
        if !e.text().is_empty() {
            st2.set_visible_child_name("ver");
        }
    });
    entry.add_controller(focus);
    let (st2, s2) = (st.clone(), sync);
    entry.connect_activate(move |e| {
        s2();
        if !e.text().is_empty() {
            st2.set_visible_child_name("ver");
        }
    });
    st
}

pub struct SandboxSection {
    pub root: gtk::Box,
    /// "Como fica", for the editor's side
    pub side: gtk::Box,
    mode: Segmented,
    settings_box: gtk::Box,
    template: gtk::DropDown,
    network: gtk::DropDown,
    on_end: gtk::DropDown,
    inputs: gtk::Box,
    outputs: gtk::Box,
    limits: [gtk::Label; 2],
    time_limit: gtk::DropDown,
    value: RefCell<Value>,
    templates: RefCell<Vec<Machine>>,
    filling: Cell<bool>,
    loaded: Cell<bool>,
    on_change: Box<dyn Fn(Value)>,
    // the side
    s_host: gtk::Label,
    s_machine: gtk::Label,
    s_can: gtk::Box,
}

impl SandboxSection {
    pub fn new(on_change: impl Fn(Value) + 'static) -> Rc<SandboxSection> {
        let root = caixa(Orientation::Vertical, 0, "");
        root.append(&ui::left(tr("Where it runs"), "fx-secao-titulo"));
        let holder: Rc<RefCell<std::rc::Weak<SandboxSection>>> = Rc::default();
        let h = holder.clone();
        let mode = Segmented::new(&[("local".into(), tr("On this computer").into()), ("sandbox".into(), tr("In a sandbox (VirtualDesk)").into())], "local", move |id| {
            if let Some(s) = h.borrow().upgrade().filter(|s| !s.filling.get()) {
                s.set_mode(id == "sandbox");
            }
        });
        mode.root.set_margin_top(12);
        root.append(&mode.root);
        let settings_box = caixa(Orientation::Vertical, 0, "");
        let note = ui::wrapped(tr("Each run starts on a clean virtual machine, from a template. Nothing of your computer goes in unless it is listed below."), "fx-nota");
        note.set_margin_top(10);
        settings_box.append(&note);
        let row = caixa(Orientation::Horizontal, 12, "");
        row.set_homogeneous(true);
        row.set_margin_top(12);
        let template = ui::dropdown(&[tr("— choose —").to_string()], 0);
        row.append(&ui::labelled(tr("Machine template"), &template));
        let network = ui::dropdown(&NETWORKS.iter().map(|n| tr(n.1).to_string()).collect::<Vec<_>>(), 0);
        row.append(&ui::labelled(tr("Sandbox network"), &network));
        let on_end = ui::dropdown(&ENDS.iter().map(|n| tr(n.1).to_string()).collect::<Vec<_>>(), 0);
        row.append(&ui::labelled(tr("After it ends"), &on_end));
        settings_box.append(&row);
        let table = |title: &str, sub: &str, cols: [&str; 3]| -> (gtk::Box, gtk::Box, gtk::Button) {
            let t = caixa(Orientation::Horizontal, 8, "");
            t.set_margin_top(20);
            let a = ui::left(title, "fx-secao-titulo");
            a.set_valign(Align::Baseline);
            t.append(&a);
            let b = ui::left(sub, "fx-secao-sub");
            b.set_valign(Align::Baseline);
            t.append(&b);
            settings_box.append(&t);
            let root = caixa(Orientation::Vertical, 0, "fx-tabela");
            root.set_margin_top(10);
            let head = caixa(Orientation::Horizontal, 12, "fx-tabela-cab");
            for (i, c) in cols.iter().enumerate() {
                let l = rotulo(c, "fx-tabela-titulo");
                l.set_xalign(0.0);
                if i < 2 {
                    l.set_hexpand(true);
                } else {
                    l.set_size_request(150, -1);
                }
                head.append(&l);
            }
            let pad = caixa(Orientation::Horizontal, 0, "");
            pad.set_size_request(28, -1);
            head.append(&pad);
            root.append(&head);
            let rows = caixa(Orientation::Vertical, 0, "");
            root.append(&rows);
            let add = ui::link(tr("Add a line"));
            add.add_css_class("fx-tabela-mais");
            add.set_halign(Align::Start);
            root.append(&add);
            settings_box.append(&root);
            (rows, root, add)
        };
        let (inputs, _, add_in) = table(tr("Inputs"), tr("copied into the machine before it starts"), [tr("FROM THE COMPUTER"), tr("INTO THE MACHINE"), tr("MODE")]);
        let (outputs, _, add_out) = table(tr("Outputs"), tr("only what is here comes back to the computer"), [tr("FROM THE MACHINE"), tr("TO THE COMPUTER"), tr("REACH")]);
        let lt = ui::left(tr("The machine's limits"), "fx-secao-titulo");
        lt.set_margin_top(20);
        settings_box.append(&lt);
        let lrow = caixa(Orientation::Horizontal, 12, "");
        lrow.set_homogeneous(true);
        lrow.set_margin_top(10);
        let lim = |t: &str| {
            let v = rotulo("—", "fx-campo-leitura");
            v.set_xalign(0.0);
            lrow.append(&ui::labelled(t, &v));
            v
        };
        let limits = [lim(tr("Processors")), lim(tr("Memory"))];
        let time_limit = ui::dropdown(&TIME_LIMITS.iter().map(|t| time_limit_text(*t)).collect::<Vec<_>>(), 0);
        lrow.append(&ui::labelled(tr("Longest time"), &time_limit));
        settings_box.append(&lrow);
        let ln = ui::wrapped(tr("Processors and memory are the template's own (a copy resumes its memory snapshot); change them in VirtualDesk. Past the longest time the run stops and the machine closes."), "fx-nota");
        ln.set_margin_top(6);
        settings_box.append(&ln);
        let share = caixa(Orientation::Horizontal, 12, "fx-chave-linha");
        share.set_margin_top(16);
        let sc = caixa(Orientation::Vertical, 2, "");
        sc.set_hexpand(true);
        sc.append(&ui::left(tr("Share the computer's folders"), "fx-chave-titulo"));
        sc.append(&ui::wrapped(tr("off: inputs and outputs are copies, the machine does not see your folders"), "fx-chave-sub"));
        share.append(&sc);
        let sw = ui::switch();
        sw.set_active(false);
        sw.set_sensitive(false);
        sw.set_valign(Align::Center);
        share.append(&sw);
        settings_box.append(&share);
        settings_box.set_visible(false);
        root.append(&settings_box);

        // "Como fica"
        let side = caixa(Orientation::Vertical, 0, "");
        side.append(&ui::panel_title("olho", tr("HOW IT LOOKS")));
        let flow = caixa(Orientation::Horizontal, 8, "central-cartao");
        flow.add_css_class("fx-sandbox-fluxo");
        flow.set_margin_top(14);
        let card = |t: &str| {
            let c = caixa(Orientation::Vertical, 2, "fx-sandbox-ponta");
            c.set_hexpand(true);
            c.append(&ui::left(t, "fx-chave-titulo"));
            let s = ui::cut("", "fx-chave-sub");
            c.append(&s);
            (c, s)
        };
        let (host, s_host) = card(tr("Your computer"));
        flow.append(&host);
        let arrows = caixa(Orientation::Vertical, 2, "");
        arrows.set_valign(Align::Center);
        arrows.append(&ui::left(&format!("{} →", tr("inputs")), "fx-sandbox-seta"));
        arrows.append(&ui::left(&format!("← {}", tr("outputs")), "fx-sandbox-seta"));
        flow.append(&arrows);
        let (machine, s_machine) = card(tr("Sandbox"));
        flow.append(&machine);
        side.append(&flow);
        let goal = caixa(Orientation::Vertical, 4, "central-cartao");
        goal.add_css_class("memoria-info");
        goal.set_margin_top(14);
        let gt = caixa(Orientation::Horizontal, 8, "");
        gt.append(&ui::glyph("raio", ui::ACCENT, 14));
        gt.append(&ui::left(tr("Goal: ready in ~3 s"), "fx-chave-titulo"));
        goal.append(&gt);
        goal.append(&ui::wrapped(tr("resuming the template's snapshot with memory; the real time shows in the run's trace once measured on this computer"), "fx-chave-sub"));
        side.append(&goal);
        let ct = ui::left(tr("WHAT THE AGENT MAY DO IN THERE"), "central-painel-secao");
        ct.set_margin_top(18);
        side.append(&ct);
        let s_can = caixa(Orientation::Vertical, 6, "central-cartao");
        s_can.add_css_class("memoria-info");
        s_can.set_margin_top(10);
        side.append(&s_can);
        let foot = ui::note("escudo", tr("Inside the sandbox the agent is free; what leaves it goes through kalangod (user level, allowed folders) and into the Timeline."));
        foot.set_margin_top(14);
        side.append(&foot);

        let s = Rc::new(SandboxSection {
            root, side, mode, settings_box, template, network, on_end, inputs, outputs, limits, time_limit, value: RefCell::new(Value::Null), templates: RefCell::default(),
            filling: Cell::new(false), loaded: Cell::new(false), on_change: Box::new(on_change), s_host, s_machine, s_can,
        });
        *holder.borrow_mut() = Rc::downgrade(&s);
        for (d, key) in [(&s.template, "template"), (&s.network, "network"), (&s.on_end, "on_end")] {
            let w = Rc::downgrade(&s);
            d.connect_selected_notify(move |d| {
                let Some(s) = w.upgrade().filter(|s| !s.filling.get()) else { return };
                let i = d.selected() as usize;
                let v = match key {
                    "template" => s.templates.borrow().get(i.wrapping_sub(1)).map(|m| m.id.clone()).unwrap_or_default(),
                    "network" => NETWORKS.get(i).map(|n| n.0.to_string()).unwrap_or_default(),
                    _ => ENDS.get(i).map(|n| n.0.to_string()).unwrap_or_default(),
                };
                s.edit(|x| x[key] = json!(v));
            });
        }
        let w = Rc::downgrade(&s);
        s.time_limit.connect_selected_notify(move |d| {
            let Some(s) = w.upgrade().filter(|s| !s.filling.get()) else { return };
            let cur = s.value.borrow()["time_limit_s"].as_u64();
            let t = time_limit_options(cur).get(d.selected() as usize).copied().flatten();
            s.edit(|x| match t {
                Some(t) => x["time_limit_s"] = json!(t),
                None => {
                    x.as_object_mut().map(|o| o.remove("time_limit_s"));
                }
            });
        });
        for (b, key) in [(add_in, "inputs"), (add_out, "outputs")] {
            let w = Rc::downgrade(&s);
            b.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.edit(|x| {
                        let mut l = x[key].as_array().cloned().unwrap_or_default();
                        l.push(json!({"from": "", "to": ""}));
                        x[key] = json!(l);
                    });
                    s.fill_tables();
                }
            });
        }
        s
    }

    /// Whether the agent runs in a sandbox.
    pub fn is_on(&self) -> bool {
        self.value.borrow().is_object()
    }

    /// Shows an agent's `config.sandbox` (null: it runs on this computer).
    pub fn set(self: &Rc<Self>, v: &Value) {
        *self.value.borrow_mut() = if v.is_object() { v.clone() } else { Value::Null };
        self.filling.set(true);
        self.mode.set(if self.is_on() { "sandbox" } else { "local" });
        self.filling.set(false);
        self.show();
        if !self.loaded.replace(true) {
            let w = Rc::downgrade(self);
            vm_call("vm.listar", json!({}), move |r| {
                if let Some(s) = w.upgrade() {
                    *s.templates.borrow_mut() = r.map(|v| machines(&v).into_iter().filter(|m| !m.is_copy()).collect()).unwrap_or_default();
                    s.show();
                }
            });
        }
    }

    fn set_mode(self: &Rc<Self>, sandbox: bool) {
        let v = if sandbox { new_settings(&self.templates.borrow().first().map(|m| m.id.clone()).unwrap_or_default()) } else { Value::Null };
        *self.value.borrow_mut() = v.clone();
        self.show();
        (self.on_change)(v);
    }

    fn edit(self: &Rc<Self>, f: impl FnOnce(&mut Value)) {
        if !self.is_on() {
            return;
        }
        f(&mut self.value.borrow_mut());
        let v = self.value.borrow().clone();
        self.show_side();
        self.show_limits();
        (self.on_change)(v);
    }

    fn show(self: &Rc<Self>) {
        self.filling.set(true);
        let v = self.value.borrow().clone();
        self.settings_box.set_visible(v.is_object());
        let cur = v["template"].as_str().unwrap_or("").to_string();
        let mut names = vec![tr("— choose —").to_string()];
        names.extend(self.templates.borrow().iter().map(|m| format!("{} ({})", m.title(), gb_text(m.disk_gb))));
        let known = self.templates.borrow().iter().position(|m| m.id == cur);
        if known.is_none() && !cur.is_empty() {
            names.push(cur.clone());
        }
        self.template.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
        self.template.set_selected(match known {
            Some(i) => i as u32 + 1,
            None if !cur.is_empty() => names.len() as u32 - 1,
            None => 0,
        });
        self.network.set_selected(NETWORKS.iter().position(|n| v["network"] == json!(n.0)).unwrap_or(0) as u32);
        self.on_end.set_selected(ENDS.iter().position(|n| v["on_end"] == json!(n.0)).unwrap_or(0) as u32);
        self.time_limit.set_model(Some(&gtk::StringList::new(&time_limit_options(v["time_limit_s"].as_u64()).iter().map(|t| time_limit_text(*t)).collect::<Vec<_>>().iter().map(String::as_str).collect::<Vec<_>>())));
        self.time_limit.set_selected(time_limit_options(v["time_limit_s"].as_u64()).iter().position(|t| *t == v["time_limit_s"].as_u64()).unwrap_or(0) as u32);
        self.filling.set(false);
        self.fill_tables();
        self.show_limits();
        self.show_side();
    }

    fn show_limits(&self) {
        let v = self.value.borrow();
        let t = self.templates.borrow();
        let m = t.iter().find(|m| v["template"] == json!(m.id));
        self.limits[0].set_text(&m.map(|m| m.cpus.to_string()).unwrap_or_else(|| "—".into()));
        self.limits[1].set_text(&m.map(|m| gb_text(m.memory_mb as f64 / 1024.0)).unwrap_or_else(|| "—".into()));
    }

    /// The input and output lines, each "from → to" with its mode or reach.
    fn fill_tables(self: &Rc<Self>) {
        for (rows, key) in [(&self.inputs, "inputs"), (&self.outputs, "outputs")] {
            ui::clear_box(rows);
            let list = self.value.borrow()[key].as_array().cloned().unwrap_or_default();
            for (i, c) in list.iter().enumerate() {
                let row = caixa(Orientation::Horizontal, 12, "fx-tabela-linha");
                let mut boxes = Vec::new();
                // (the two paths share the room equally, as the header's columns do)
                let paths = caixa(Orientation::Horizontal, 12, "");
                paths.set_homogeneous(true);
                paths.set_hexpand(true);
                row.append(&paths);
                for (k, hint) in [("from", if key == "inputs" { "{anexo}" } else { "/saida/relatorio.md" }), ("to", if key == "inputs" { "/entrada/" } else { "~/Documentos/Analises/" })] {
                    let e = ui::mono_entry(c[k].as_str().unwrap_or(""), hint);
                    e.add_css_class("fx-tabela-campo");
                    e.set_width_chars(1);
                    paths.append(&path_cell(&e));
                    boxes.push((k, e));
                }
                let third = if key == "inputs" { ui::chip(tr("copy"), "azul") } else { ui::left(&output_reach(c["from"].as_str().unwrap_or("")), "fx-chave-sub") };
                let tb = caixa(Orientation::Horizontal, 0, "");
                tb.set_size_request(150, -1);
                third.set_valign(Align::Center);
                tb.append(&third);
                row.append(&tb);
                let rm = gtk::Button::new();
                rm.add_css_class("fx-tabela-menos");
                rm.set_child(Some(&ui::glyph("lixeira", ui::MUTED, 14)));
                rm.set_tooltip_text(Some(tr("Remove")));
                rm.set_valign(Align::Center);
                row.append(&rm);
                for (k, e) in boxes {
                    let w = Rc::downgrade(self);
                    e.connect_changed(move |e| {
                        if let Some(s) = w.upgrade() {
                            let t = e.text().trim().to_string();
                            s.edit(|x| x[key][i][k] = json!(t));
                        }
                    });
                }
                let w = Rc::downgrade(self);
                rm.connect_clicked(move |_| {
                    if let Some(s) = w.upgrade() {
                        s.edit(|x| {
                            if let Some(a) = x[key].as_array_mut() {
                                if i < a.len() {
                                    a.remove(i);
                                }
                            }
                        });
                        s.fill_tables();
                    }
                });
                rows.append(&row);
            }
        }
    }

    fn show_side(&self) {
        let v = self.value.borrow().clone();
        let folders = host_folders(&v);
        self.s_host.set_text(if folders.is_empty() { tr("nothing goes in") } else { &folders });
        let t = self.templates.borrow();
        let name = t.iter().find(|m| v["template"] == json!(m.id)).map(Machine::title).unwrap_or_else(|| v["template"].as_str().unwrap_or("").to_string());
        let net = m::SANDBOX_NETWORKS.iter().find(|n| v["network"] == json!(n.0)).map(|n| tr(n.2)).unwrap_or(tr("no network"));
        self.s_machine.set_text(&format!("{name} · {net}"));
        ui::clear_box(&self.s_can);
        let online = v["network"].as_str().is_some_and(|n| n != "nenhuma");
        for (what, free) in [(tr("vm.executar (commands)"), true), (tr("install packages"), online), (tr("browse the web"), online), (tr("copy out what is not in Outputs"), false)] {
            let r = caixa(Orientation::Horizontal, 8, "");
            let l = ui::left(what, "fx-chave-sub");
            l.set_hexpand(true);
            r.append(&l);
            r.append(&if free { ui::chip(tr("free"), "teal") } else { ui::chip(tr("never"), "coral") });
            self.s_can.append(&r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_crosses_reads_as_folders_and_reach() {
        let s = json!({"inputs": [{"from": "{anexo}", "to": "/entrada/"}, {"from": "~/Documentos/regras.txt", "to": "/entrada/regras.txt"}],
                       "outputs": [{"from": "/saida/limpo/*", "to": "~/Downloads/verificados/"}]});
        assert_eq!(host_folders(&s), "~/Documentos · ~/Downloads");
        assert_eq!(output_reach("/saida/limpo/*"), tr("every file of the folder"));
        assert_eq!(output_reach("/saida/relatorio.md"), tr("1 file"));
        assert_eq!(new_settings("t")["network"], json!("nenhuma"));
        assert_eq!(time_limit_text(Some(600)), "10 min");
        assert_eq!(time_limit_text(Some(3600)), "1 h");
        assert_eq!(time_limit_text(Some(90)), "90 s");
        assert_eq!(time_limit_options(Some(90)).last(), Some(&Some(90)));
        assert_eq!(time_limit_options(Some(600)).len(), TIME_LIMITS.len());
    }
}
