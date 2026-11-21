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

    impl WidgetImpl for Limite {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, o: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(c) = self.filho.borrow().clone() else { return (0, 0, -1, -1) };
            let max = self.max.get();
            if o == gtk::Orientation::Horizontal {
                let (min, nat, _, _) = c.measure(o, for_size);
                let nat = if self.fill.get() { max } else { nat.min(max) };
                (min, nat.max(min), -1, -1)
            } else {
                // the height for the width the child will get
                let w = if for_size < 0 { -1 } else { for_size.min(max).max(c.measure(gtk::Orientation::Horizontal, -1).0) };
                let (min, nat, _, _) = c.measure(o, w);
                (min, nat, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(c) = self.filho.borrow().as_ref() {
                let min = c.measure(gtk::Orientation::Horizontal, height).0;
                let w = width.min(self.max.get()).max(min);
                c.size_allocate(&gtk::Allocation::new(((width - w) / 2).max(0), 0, w, height), baseline);
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

    /// A new limit (the window changed size).
    pub fn set_max(&self, max: i32) {
        if self.imp().max.replace(max) != max {
            self.queue_resize();
        }
    }
}
