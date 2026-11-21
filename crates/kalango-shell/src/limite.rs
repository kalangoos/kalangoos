//! A width limit that gives way: the child is at most `max` pixels wide and centred when there is more room, and as
//! narrow as its own minimum when there is less (a fixed size request would hold the whole window wide instead).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct Limite {
        pub max: Cell<i32>,
        /// asks for `max` even when the child would be happy narrower (a side panel that should keep its width)
        pub fill: Cell<bool>,
        /// the child's width from the width given (a share of it, `widths::reading_column`) instead of `max`
        pub rule: Cell<Option<fn(i32) -> i32>>,
        /// a measure in characters of the child's own font (> 0) instead of `max`, the child at the start
        pub chars: Cell<i32>,
        pub filho: RefCell<Option<gtk::Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Limite {
        const NAME: &'static str = "KalangoLimite";
        type Type = super::Limite;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Limite {
        fn dispose(&self) {
            if let Some(c) = self.filho.take() {
                c.unparent();
            }
        }
    }

    impl Limite {
        /// The widest the child may be: `chars` of its font (the wider of a letter and a digit, as GTK counts
        /// `max-width-chars`; read at every layout, so it follows the font size and scale), else `max`.
        pub fn widest(&self, c: &gtk::Widget) -> i32 {
            let n = self.chars.get();
            if n <= 0 {
                return self.max.get();
            }
            let m = c.pango_context().metrics(None, None);
            n * (m.approximate_char_width().max(m.approximate_digit_width()) / gtk::pango::SCALE).max(1)
        }
    }

    impl WidgetImpl for Limite {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, o: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(c) = self.filho.borrow().clone() else { return (0, 0, -1, -1) };
            let max = self.widest(&c);
            let rule = self.rule.get();
            if o == gtk::Orientation::Horizontal {
                let (min, nat, _, _) = c.measure(o, for_size);
                // (with a rule the width comes from the room given: ask for no more than the child's least, the
                // limit expands into the rest)
                let nat = if rule.is_some() { min } else if self.fill.get() { max } else { nat.min(max) };
                (min, nat.max(min), -1, -1)
            } else {
                // the height for the width the child will get
                let width = |w: i32| match rule { Some(r) => r(w), None => w.min(max) };
                let w = if for_size < 0 { -1 } else { width(for_size).max(c.measure(gtk::Orientation::Horizontal, -1).0) };
                let (min, nat, _, _) = c.measure(o, w);
                (min, nat, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(c) = self.filho.borrow().as_ref() {
                let min = c.measure(gtk::Orientation::Horizontal, height).0;
                let w = match self.rule.get() { Some(r) => r(width), None => width.min(self.widest(c)) }.max(min);
                let x = if self.chars.get() > 0 { 0 } else { ((width - w) / 2).max(0) };
                c.size_allocate(&gtk::Allocation::new(x, 0, w, height), baseline);
            }
        }
    }
}

glib::wrapper! {
    pub struct Limite(ObjectSubclass<imp::Limite>) @extends gtk::Widget;
}

impl Limite {
    /// `filho` at most `max` px wide; the limit itself takes all the room it is given (hexpand).
    pub fn new(filho: &impl IsA<gtk::Widget>, max: i32) -> Limite {
        let l: Limite = glib::Object::new();
        l.imp().max.set(max);
        filho.set_parent(&l);
        *l.imp().filho.borrow_mut() = Some(filho.clone().upcast());
        l.set_hexpand(true);
        l
    }

    /// Asks for the whole limit, not only what the child would like (it still gives way down to the child's
    /// minimum).
    pub fn set_fill(&self, fill: bool) {
        self.imp().fill.set(fill);
        self.queue_resize();
    }

    /// The child's width follows `rule` (the width given → the child's), recomputed at every allocation: no fixed
    /// ceiling, and no jump while the window or a panel is resized.
    pub fn with_rule(filho: &impl IsA<gtk::Widget>, rule: fn(i32) -> i32) -> Limite {
        let l = Limite::new(filho, i32::MAX);
        l.imp().rule.set(Some(rule));
        l
    }

    /// Prose: `filho` at most `chars` characters of its own font wide, at the start of the room (a paragraph in a
    /// column that may be much wider — design "Central · conversa responsiva").
    pub fn measure_in_chars(filho: &impl IsA<gtk::Widget>, chars: i32) -> Limite {
        let l = Limite::new(filho, i32::MAX);
        l.imp().chars.set(chars);
        // (it expands only where its text did: a bubble or a row keeps its own layout)
        l.set_hexpand(filho.hexpands());
        l
    }

    /// A new limit (the window changed size).
    pub fn set_max(&self, max: i32) {
        if self.imp().max.replace(max) != max {
            self.queue_resize();
        }
    }
}
