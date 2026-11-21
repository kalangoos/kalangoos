//! The agent editor's structured parts: the task list of a "Tarefas" agent (design 41, light 47), the structured
//! input and output tables of the Avançado tab (design 42, light 48) and the "Testar agora" form generated from the
//! input fields. kalango-fluxos keeps them as `config.tasks: [{agent, description, expected_output, tools}]` and
//! the agent's top-level `input_fields` / `output_fields: [{name, kind, required, description}]`.

use crate::agent_picker;
use crate::fluxos_ui as ui;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// The kinds a field takes (kalango-fluxos `fields::KINDS`) and how the person reads them.
pub const FIELD_KINDS: [(&str, &str); 6] = [
    ("text", n_("Text")),
    ("number", n_("Number")),
    ("integer", n_("Whole number")),
    ("boolean", n_("Yes or no")),
    ("list", n_("List")),
    ("object", n_("Object")),
];

/// Whether a name can be a field (and so `{name}` in the instructions): letters, digits and _, not starting with a
/// digit.
pub fn is_field_name(name: &str) -> bool {
    let mut c = name.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// What keeps a field list from being saved, as the person reads it.
pub fn fields_issues(fields: &Value) -> Vec<String> {
    let mut v = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for f in fields.as_array().into_iter().flatten() {
        let name = f["name"].as_str().unwrap_or("");
        if !is_field_name(name) {
            v.push(trf("The field “{name}” takes letters, digits and _.", &[("name", name.to_string())]));
        } else if seen.contains(&name) {
            v.push(trf("The field “{name}” appears twice.", &[("name", name.to_string())]));
        }
        seen.push(name);
    }
    v
}

/// The JSON Schema the model answers in, from the output fields ("Ver JSON Schema").
pub fn json_schema(fields: &Value) -> Value {
    let mut props = Map::new();
    let mut required = Vec::new();
    for f in fields.as_array().into_iter().flatten() {
        let name = f["name"].as_str().unwrap_or("").to_string();
        let t = match f["kind"].as_str().unwrap_or("text") {
            "number" => "number",
            "integer" => "integer",
            "boolean" => "boolean",
            "list" => "array",
            "object" => "object",
            _ => "string",
        };
        let mut p = json!({"type": t});
        if let Some(d) = f["description"].as_str().filter(|d| !d.is_empty()) {
            p["description"] = json!(d);
        }
        if f["required"].as_bool().unwrap_or(false) {
            required.push(json!(name));
        }
        props.insert(name, p);
    }
    json!({"type": "object", "properties": props, "required": required})
}

/// What keeps a task list from being saved (the service checks the same).
pub fn tasks_issues(tasks: &Value) -> Vec<String> {
    let list = tasks.as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        return vec![tr("Add at least one task.").to_string()];
    }
    let mut v = Vec::new();
    for (i, t) in list.iter().enumerate() {
        let n = (i + 1).to_string();
        if t["agent"].as_str().unwrap_or("").is_empty() {
            v.push(trf("Task {n}: choose who does it.", &[("n", n.clone())]));
        }
        if t["description"].as_str().unwrap_or("").trim().is_empty() {
            v.push(trf("Task {n}: say what to do.", &[("n", n.clone())]));
        }
        if t["expected_output"].as_str().unwrap_or("").trim().is_empty() {
            v.push(trf("Task {n}: say what it hands back.", &[("n", n)]));
        }
    }
    v
}

/// The tools of a task as its chip says them: all of the assistant's (an empty list), the chosen ones, or none
/// when the assistant has none.
pub fn task_tools_text(chosen: &[String], agent_tools: &[String]) -> String {
    if agent_tools.is_empty() {
        tr("none").to_string()
    } else if chosen.is_empty() {
        tr("all of its own").to_string()
    } else {
        chosen.join(", ")
    }
}

fn agent_tools(agent: &Value) -> Vec<String> {
    agent["config"]["tools"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str().map(str::to_string)).collect()
}

// ------------------------------------------------------------------------------------------------ fields table

struct FieldRow {
    root: gtk::Box,
    name: gtk::Entry,
    kind: gtk::DropDown,
    required: gtk::Switch,
    description: gtk::Entry,
}

/// A table of fields (name, kind, required, description) with "+ Adicionar campo".
pub struct FieldsTable {
    pub root: gtk::Box,
    rows_box: gtk::Box,
    rows: RefCell<Vec<FieldRow>>,
    on_change: Box<dyn Fn()>,
}

impl FieldsTable {
    pub fn new(on_change: impl Fn() + 'static) -> Rc<FieldsTable> {
        let root = caixa(Orientation::Vertical, 0, "fx-tabela");
        let head = caixa(Orientation::Horizontal, 12, "fx-tabela-cab");
        for (t, w) in [(tr("NAME"), 150), (tr("TYPE"), 140), (tr("REQUIRED"), 84)] {
            let l = rotulo(t, "fx-tabela-titulo");
            l.set_xalign(0.0);
            l.set_size_request(w, -1);
            head.append(&l);
        }
        let d = rotulo(tr("DESCRIPTION"), "fx-tabela-titulo");
        d.set_xalign(0.0);
        d.set_hexpand(true);
        head.append(&d);
        root.append(&head);
        let rows_box = caixa(Orientation::Vertical, 0, "");
        root.append(&rows_box);
        let add = ui::link(tr("Add field"));
        add.add_css_class("fx-tabela-mais");
        add.set_halign(Align::Start);
        root.append(&add);
        let t = Rc::new(FieldsTable { root, rows_box, rows: RefCell::default(), on_change: Box::new(on_change) });
        let w = Rc::downgrade(&t);
        add.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.add_row(&json!({"name": "", "kind": "text", "required": false, "description": ""}));
                if let Some(r) = t.rows.borrow().last() {
                    r.name.grab_focus();
                }
            }
        });
        t
    }

    /// Shows these fields (the agent's `input_fields` or `output_fields`).
    pub fn set(self: &Rc<Self>, fields: &Value) {
        for r in self.rows.borrow_mut().drain(..) {
            self.rows_box.remove(&r.root);
        }
        for f in fields.as_array().into_iter().flatten() {
            self.add_row(f);
        }
    }

    /// The fields as kalango-fluxos keeps them (rows with no name are not fields yet).
    pub fn value(&self) -> Value {
        Value::Array(
            self.rows
                .borrow()
                .iter()
                .filter(|r| !r.name.text().trim().is_empty())
                .map(|r| {
                    json!({
                        "name": r.name.text().trim(),
                        "kind": FIELD_KINDS.get(r.kind.selected() as usize).map(|k| k.0).unwrap_or("text"),
                        "required": r.required.is_active(),
                        "description": r.description.text().trim(),
                    })
                })
                .collect(),
        )
    }

    fn add_row(self: &Rc<Self>, f: &Value) {
        let root = caixa(Orientation::Horizontal, 12, "fx-tabela-linha");
        let name = ui::mono_entry(f["name"].as_str().unwrap_or(""), "nome");
        name.add_css_class("fx-tabela-campo");
        // (the columns' widths are the header's: the entries ask for no more)
        name.set_width_chars(1);
        name.set_size_request(150, -1);
        name.set_hexpand(false);
        root.append(&name);
        let kinds: Vec<String> = FIELD_KINDS.iter().map(|(_, l)| tr(l).to_string()).collect();
        let at = FIELD_KINDS.iter().position(|(k, _)| Some(*k) == f["kind"].as_str()).unwrap_or(0);
        let kind = ui::dropdown(&kinds, at);
        kind.set_size_request(140, -1);
        kind.set_hexpand(false);
        kind.set_valign(Align::Center);
        root.append(&kind);
        let sw_box = caixa(Orientation::Horizontal, 0, "");
        sw_box.set_size_request(84, -1);
        let required = ui::switch();
        required.set_active(f["required"].as_bool().unwrap_or(false));
        required.set_valign(Align::Center);
        sw_box.append(&required);
        root.append(&sw_box);
        let description = ui::entry(f["description"].as_str().unwrap_or(""), "");
        description.add_css_class("fx-tabela-campo");
        description.set_width_chars(1);
        description.set_hexpand(true);
        root.append(&description);
        let remove = gtk::Button::new();
        remove.add_css_class("fx-tabela-menos");
        remove.set_child(Some(&ui::glyph("lixeira", ui::MUTED, 14)));
        remove.set_tooltip_text(Some(tr("Remove")));
        remove.set_valign(Align::Center);
        root.append(&remove);
        self.rows_box.append(&root);
        let w = Rc::downgrade(self);
        let changed = move || {
            if let Some(t) = w.upgrade() {
                (t.on_change)();
            }
        };
        let c = changed.clone();
        name.connect_changed(move |_| c());
        let c = changed.clone();
        kind.connect_selected_notify(move |_| c());
        let c = changed.clone();
        required.connect_active_notify(move |_| c());
        let c = changed.clone();
        description.connect_changed(move |_| c());
        let w = Rc::downgrade(self);
        let row = root.clone();
        remove.connect_clicked(move |_| {
            let Some(t) = w.upgrade() else { return };
            t.rows.borrow_mut().retain(|r| r.root != row);
            t.rows_box.remove(&row);
            (t.on_change)();
        });
        self.rows.borrow_mut().push(FieldRow { root, name, kind, required, description });
    }
}

// ------------------------------------------------------------------------------------------------ task list

struct TaskRow {
    root: gtk::Box,
    number: gtk::Label,
    agent: gtk::DropDown,
    tile: gtk::Box,
    description: gtk::Entry,
    expected: gtk::Entry,
    tools: RefCell<Vec<String>>,
    tools_button: gtk::MenuButton,
}

/// The tasks of a "Tarefas" agent: each one an assistant, what to do and what to hand back, and the tools it may
/// use in that task (fewer than the assistant's own, never more).
pub struct TasksEditor {
    pub root: gtk::Box,
    pub count: gtk::Label,
    list: gtk::Box,
    rows: RefCell<Vec<Rc<TaskRow>>>,
    /// the assistants that can take a task
    assistants: RefCell<Vec<Value>>,
    on_change: Box<dyn Fn()>,
}

impl TasksEditor {
    pub fn new(on_change: impl Fn() + 'static) -> Rc<TasksEditor> {
        let root = caixa(Orientation::Vertical, 0, "");
        let head = caixa(Orientation::Horizontal, 8, "");
        head.append(&rotulo(tr("Tasks"), "fx-secao-titulo"));
        let count = ui::left("", "fx-secao-sub");
        count.set_hexpand(true);
        count.set_valign(Align::Baseline);
        head.append(&count);
        let add = ui::button(Some("mais"), tr("Add task"), false);
        head.append(&add);
        root.append(&head);
        let list = caixa(Orientation::Vertical, 8, "");
        list.set_margin_top(12);
        root.append(&list);
        let note = ui::note("info", tr("If a task fails, the next ones do not run. Only assistants take tasks; a task can limit the assistant's tools, never add to them."));
        note.set_margin_top(12);
        root.append(&note);
        let t = Rc::new(TasksEditor { root, count, list, rows: RefCell::default(), assistants: RefCell::default(), on_change: Box::new(on_change) });
        let w = Rc::downgrade(&t);
        add.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.add_row(&json!({"agent": "", "description": "", "expected_output": "", "tools": []}));
                t.renumber();
                (t.on_change)();
            }
        });
        t
    }

    /// Shows these tasks; `agents` are the person's agents (only assistants take tasks).
    pub fn set(self: &Rc<Self>, tasks: &Value, agents: &[Value]) {
        *self.assistants.borrow_mut() = agents.iter().filter(|a| ui::kind_of(a) == "assistant").cloned().collect();
        for r in self.rows.borrow_mut().drain(..) {
            self.list.remove(&r.root);
        }
        for t in tasks.as_array().into_iter().flatten() {
            self.add_row(t);
        }
        self.renumber();
    }

    pub fn value(&self) -> Value {
        let assistants = self.assistants.borrow();
        Value::Array(
            self.rows
                .borrow()
                .iter()
                .map(|r| {
                    let agent = assistants.get(r.agent.selected() as usize).and_then(|a| a["id"].as_str()).unwrap_or("");
                    json!({
                        "agent": agent,
                        "description": r.description.text().trim(),
                        "expected_output": r.expected.text().trim(),
                        "tools": *r.tools.borrow(),
                    })
                })
                .collect(),
        )
    }

    fn renumber(&self) {
        let rows = self.rows.borrow();
        for (i, r) in rows.iter().enumerate() {
            r.number.set_text(&(i + 1).to_string());
        }
        let n = rows.len();
        self.count.set_text(&if n == 0 {
            tr("none yet").to_string()
        } else {
            trf("{n} · in order; each one sees the answers before it", &[("n", n.to_string())])
        });
    }

    fn agent_of(&self, r: &TaskRow) -> Value {
        self.assistants.borrow().get(r.agent.selected() as usize).cloned().unwrap_or(Value::Null)
    }

    fn add_row(self: &Rc<Self>, t: &Value) {
        let root = caixa(Orientation::Vertical, 0, "fx-tarefa");
        let top = caixa(Orientation::Horizontal, 10, "");
        top.append(&ui::glyph("menu", ui::MUTED, 14));
        let number = rotulo("", "fx-passo-numero");
        top.append(&number);
        let tile = caixa(Orientation::Horizontal, 0, "");
        top.append(&tile);
        let names: Vec<String> = self.assistants.borrow().iter().map(|a| a["name"].as_str().unwrap_or("").to_string()).collect();
        let at = self.assistants.borrow().iter().position(|a| a["id"].as_str() == t["agent"].as_str()).unwrap_or(0);
        let agent = ui::dropdown(&names, at);
        agent.set_size_request(220, -1);
        agent.set_hexpand(false);
        agent.set_valign(Align::Center);
        top.append(&agent);
        let kind = rotulo(tr("assistant"), "fx-secao-sub");
        kind.set_hexpand(true);
        kind.set_xalign(0.0);
        top.append(&kind);
        let menu = gtk::MenuButton::new();
        menu.add_css_class("fx-menu-pontos");
        menu.set_child(Some(&ui::glyph("pontos-v", ui::MUTED, 14)));
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let pl = caixa(Orientation::Vertical, 2, "");
        let up = gtk::Button::with_label(tr("Move up"));
        let down = gtk::Button::with_label(tr("Move down"));
        let rm = gtk::Button::with_label(tr("Remove"));
        for b in [&up, &down, &rm] {
            b.add_css_class("central-menu-item");
            pl.append(b);
        }
        pop.set_child(Some(&pl));
        menu.set_popover(Some(&pop));
        top.append(&menu);
        root.append(&top);
        let fields = caixa(Orientation::Horizontal, 12, "");
        fields.set_margin_top(10);
        let description = ui::entry(t["description"].as_str().unwrap_or(""), "");
        let dl = ui::labelled(tr("What to do"), &description);
        dl.set_hexpand(true);
        fields.append(&dl);
        let expected = ui::entry(t["expected_output"].as_str().unwrap_or(""), "");
        let el = ui::labelled(tr("What to deliver"), &expected);
        el.set_size_request(300, -1);
        fields.append(&el);
        root.append(&fields);
        let tools_row = caixa(Orientation::Horizontal, 8, "");
        tools_row.set_margin_top(10);
        tools_row.append(&rotulo(tr("Tools in this task"), "fx-rotulo-linha"));
        let tools_button = gtk::MenuButton::new();
        tools_button.add_css_class("fx-escolha-chip");
        tools_row.append(&tools_button);
        root.append(&tools_row);
        self.list.append(&root);
        let tools: Vec<String> = t["tools"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
        let r = Rc::new(TaskRow { root, number, agent, tile, description, expected, tools: RefCell::new(tools), tools_button });
        self.rows.borrow_mut().push(r.clone());
        self.show_agent(&r);
        let w = Rc::downgrade(self);
        let changed = move || {
            if let Some(t) = w.upgrade() {
                (t.on_change)();
            }
        };
        let c = changed.clone();
        r.description.connect_changed(move |_| c());
        let c = changed.clone();
        r.expected.connect_changed(move |_| c());
        let (w, wr) = (Rc::downgrade(self), Rc::downgrade(&r));
        r.agent.connect_selected_notify(move |_| {
            let (Some(t), Some(r)) = (w.upgrade(), wr.upgrade()) else { return };
            // (another assistant: its own tools, all of them)
            r.tools.borrow_mut().clear();
            t.show_agent(&r);
            (t.on_change)();
        });
        for (b, delta) in [(&up, -1i32), (&down, 1), (&rm, 0)] {
            let (w, wr, pop) = (Rc::downgrade(self), Rc::downgrade(&r), pop.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                let (Some(t), Some(r)) = (w.upgrade(), wr.upgrade()) else { return };
                t.move_row(&r, delta);
            });
        }
    }

    /// Up (-1), down (1) or out (0).
    fn move_row(&self, r: &Rc<TaskRow>, delta: i32) {
        let mut rows = self.rows.borrow_mut();
        let Some(i) = rows.iter().position(|x| Rc::ptr_eq(x, r)) else { return };
        if delta == 0 {
            self.list.remove(&r.root);
            rows.remove(i);
        } else {
            let j = i as i32 + delta;
            if j < 0 || j as usize >= rows.len() {
                return;
            }
            rows.swap(i, j as usize);
            for x in rows.iter() {
                self.list.remove(&x.root);
            }
            for x in rows.iter() {
                self.list.append(&x.root);
            }
        }
        drop(rows);
        self.renumber();
        (self.on_change)();
    }

    /// The row's tile and its tools' chip and menu, for the assistant chosen.
    fn show_agent(self: &Rc<Self>, r: &Rc<TaskRow>) {
        let a = self.agent_of(r);
        ui::clear_box(&r.tile);
        r.tile.append(&agent_picker::icon_tile(&a, 26));
        let own = agent_tools(&a);
        r.tools_button.set_label(&task_tools_text(&r.tools.borrow(), &own));
        r.tools_button.set_sensitive(!own.is_empty());
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let pl = caixa(Orientation::Vertical, 2, "");
        pl.append(&ui::wrapped(tr("Unmark all to let it use all of its own."), "fx-nota"));
        for name in own.iter() {
            let c = gtk::CheckButton::with_label(name);
            c.set_active(r.tools.borrow().contains(name));
            let (w, wr, name, own) = (Rc::downgrade(self), Rc::downgrade(r), name.clone(), own.clone());
            c.connect_toggled(move |c| {
                let (Some(t), Some(r)) = (w.upgrade(), wr.upgrade()) else { return };
                {
                    let mut tools = r.tools.borrow_mut();
                    tools.retain(|x| *x != name);
                    if c.is_active() {
                        tools.push(name.clone());
                    }
                    // (in the assistant's own order)
                    tools.sort_by_key(|x| own.iter().position(|o| o == x));
                }
                r.tools_button.set_label(&task_tools_text(&r.tools.borrow(), &own));
                (t.on_change)();
            });
            pl.append(&c);
        }
        pop.set_child(Some(&pl));
        r.tools_button.set_popover(Some(&pop));
    }
}

// ------------------------------------------------------------------------------------------------ test form

/// "Testar agora" for an agent with a structured input: a form from its fields (design 42's right panel).
pub struct TestForm {
    pub root: gtk::Box,
    fields_box: gtk::Box,
    fields: RefCell<Vec<(agent_picker::Field, gtk::Widget, gtk::Label)>>,
    pub run: gtk::Button,
}

impl TestForm {
    pub fn new() -> Rc<TestForm> {
        let root = caixa(Orientation::Vertical, 0, "fx-teste-form");
        let fields_box = caixa(Orientation::Vertical, 6, "");
        root.append(&fields_box);
        let run = ui::button(Some("tocar"), tr("Run test"), true);
        run.set_margin_top(12);
        root.append(&run);
        Rc::new(TestForm { root, fields_box, fields: RefCell::default(), run })
    }

    pub fn is_empty(&self) -> bool {
        self.fields.borrow().is_empty()
    }

    /// A field per input field (a switch for yes or no), keeping what was typed in fields that stay.
    pub fn set_fields(&self, fields: &Value) {
        let before = self.raw_values();
        ui::clear_box(&self.fields_box);
        let mut v = Vec::new();
        for f in agent_picker::fields(fields) {
            let head = caixa(Orientation::Horizontal, 6, "");
            let name = rotulo(&f.name, "fx-teste-campo");
            head.append(&name);
            head.append(&rotulo(&format!("·  {}", agent_picker::kind_name(&f.kind)), "fx-teste-tipo"));
            let err = ui::error_label();
            let w: gtk::Widget = if f.kind == "boolean" {
                let s = ui::switch();
                s.set_halign(Align::End);
                s.set_hexpand(true);
                if let Some((_, agent_picker::Raw::Switch(b))) = before.iter().find(|(n, _)| *n == f.name) {
                    s.set_active(*b);
                }
                head.append(&s);
                self.fields_box.append(&head);
                s.upcast()
            } else {
                self.fields_box.append(&head);
                let e = if f.kind == "text" { ui::mono_entry("", &f.description) } else { ui::entry("", &f.description) };
                if let Some((_, agent_picker::Raw::Text(t))) = before.iter().find(|(n, _)| *n == f.name) {
                    e.set_text(t);
                }
                self.fields_box.append(&e);
                e.upcast()
            };
            self.fields_box.append(&err);
            v.push((f, w, err));
        }
        *self.fields.borrow_mut() = v;
        self.root.set_visible(!self.is_empty());
    }

    fn raw_values(&self) -> Vec<(String, agent_picker::Raw)> {
        self.fields
            .borrow()
            .iter()
            .map(|(f, w, _)| {
                let r = match w.downcast_ref::<gtk::Switch>() {
                    Some(s) => agent_picker::Raw::Switch(s.is_active()),
                    None => agent_picker::Raw::Text(w.downcast_ref::<gtk::Entry>().map(|e| e.text().to_string()).unwrap_or_default()),
                };
                (f.name.clone(), r)
            })
            .collect()
    }

    /// The typed input, or None with each problem under its field.
    pub fn value(&self) -> Option<Value> {
        let fs = self.fields.borrow();
        let list: Vec<agent_picker::Field> = fs.iter().map(|(f, _, _)| f.clone()).collect();
        for (_, w, e) in fs.iter() {
            ui::show_error(e, "");
            w.remove_css_class("erro");
        }
        match agent_picker::build_input(&list, &self.raw_values()) {
            Ok(v) => Some(v),
            Err(problems) => {
                for p in &problems {
                    if let Some((_, w, e)) = fs.iter().find(|(f, _, _)| f.name == p.field) {
                        let t = if p.problem == "missing" {
                            tr("fill it in").to_string()
                        } else {
                            trf("must be {kind}", &[("kind", agent_picker::kind_name(&p.kind).to_string())])
                        };
                        ui::show_error(e, &t);
                        w.add_css_class("erro");
                    }
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_have_variable_names_once() {
        assert!(is_field_name("pasta") && is_field_name("total_2") && is_field_name("_x"));
        assert!(!is_field_name("") && !is_field_name("2x") && !is_field_name("minha pasta"));
        let f = json!([{"name": "pasta"}, {"name": "pasta"}, {"name": "a b"}]);
        assert_eq!(fields_issues(&f).len(), 2);
        assert!(fields_issues(&json!([{"name": "dias"}])).is_empty());
    }

    #[test]
    fn the_output_fields_are_a_json_schema() {
        let f = json!([{"name": "movidos", "kind": "integer", "required": true, "description": "quantos"}, {"name": "pastas", "kind": "list"}]);
        let s = json_schema(&f);
        assert_eq!(s["properties"]["movidos"], json!({"type": "integer", "description": "quantos"}));
        assert_eq!(s["properties"]["pastas"]["type"], "array");
        assert_eq!(s["required"], json!(["movidos"]));
    }

    #[test]
    fn a_task_says_who_what_and_what_back() {
        assert_eq!(tasks_issues(&json!([])), vec![tr("Add at least one task.").to_string()]);
        let ok = json!([{"agent": "buscador", "description": "Junte", "expected_output": "uma tabela"}]);
        assert!(tasks_issues(&ok).is_empty());
        let bad = json!([{"agent": "", "description": " ", "expected_output": ""}]);
        assert_eq!(tasks_issues(&bad).len(), 3);
        assert_eq!(task_tools_text(&[], &["arquivos.ler".into()]), tr("all of its own"));
        assert_eq!(task_tools_text(&["arquivos.ler".into()], &["arquivos.ler".into(), "web.ler".into()]), "arquivos.ler");
        assert_eq!(task_tools_text(&[], &[]), tr("none"));
    }
}
