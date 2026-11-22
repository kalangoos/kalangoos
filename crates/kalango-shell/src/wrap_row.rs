//! A row that wraps: its children side by side at their natural width, and a new line when the next one does not fit
//! (the chips of "Nunca aprova sozinho", design page 31). Unlike a flow box, the children keep their own widths
//! (no aligned columns).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// Where each child goes in `avail` px: `sizes` are their natural (width, height), `gap` the space between two on a
/// line and `line_gap` between lines. Returns the (x, y) of each and the total height. A child wider than the line
/// takes a line of its own (and is cut to the line by the caller).
pub fn wrap(sizes: &[(i32, i32)], gap: i32, line_gap: i32, avail: i32) -> (Vec<(i32, i32)>, i32) {
    let mut at = Vec::with_capacity(sizes.len());
    let (mut x, mut y, mut line_h) = (0, 0, 0);
    for &(w, h) in sizes {
        if x > 0 && x + w > avail {
            y += line_h + line_gap;
            x = 0;
            line_h = 0;
        }
        at.push((x, y));
        x += w + gap;
        line_h = line_h.max(h);
    }
    (at, if sizes.is_empty() { 0 } else { y + line_h })
}

mod imp {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    pub struct WrapRow {
        pub gap: Cell<i32>,
        pub line_gap: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WrapRow {
        const NAME: &'static str = "KalangoWrapRow";
        type Type = super::WrapRow;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for WrapRow {}

    fn children(w: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut v = Vec::new();
        let mut c = w.first_child();
        while let Some(x) = c {
            c = x.next_sibling();
            if x.is_visible() {
                v.push(x);
            }
        }
        v
    }

    impl LayoutManagerImpl for WrapRow {
        fn request_mode(&self, _: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, widget: &gtk::Widget, o: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let kids = children(widget);
            let (gap, line_gap) = (self.gap.get(), self.line_gap.get());
            if o == gtk::Orientation::Horizontal {
                // the least: the widest child (it wraps from there); the natural: all on one line
                let sizes: Vec<(i32, i32)> = kids.iter().map(|c| {
                    let (m, n, _, _) = c.measure(o, -1);
                    (m, n)
                }).collect();
                let min = sizes.iter().map(|s| s.1).max().unwrap_or(0);
                let nat = sizes.iter().map(|s| s.1).sum::<i32>() + gap * (sizes.len() as i32 - 1).max(0);
                return (min, nat.max(min), -1, -1);
            }
            let sizes: Vec<(i32, i32)> = kids.iter().map(|c| (c.measure(gtk::Orientation::Horizontal, -1).1, c.measure(o, -1).1)).collect();
            let avail = if for_size < 0 { i32::MAX / 4 } else { for_size };
            let h = wrap(&sizes, gap, line_gap, avail).1;
            (h, h, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, _height: i32, _baseline: i32) {
            let kids = children(widget);
            let sizes: Vec<(i32, i32)> = kids.iter().map(|c| (c.measure(gtk::Orientation::Horizontal, -1).1, c.measure(gtk::Orientation::Vertical, -1).1)).collect();
            let (at, _) = wrap(&sizes, self.gap.get(), self.line_gap.get(), width);
            for ((c, (x, y)), (w, h)) in kids.iter().zip(at).zip(sizes) {
                c.size_allocate(&gtk::Allocation::new(x, y, w.min(width.max(1)), h), -1);
            }
        }
    }
}

glib::wrapper! {
    pub struct WrapRow(ObjectSubclass<imp::WrapRow>) @extends gtk::LayoutManager;
}

impl WrapRow {
    /// Lays `row`'s children out in lines that wrap, `gap` px apart on a line and `line_gap` between lines.
    pub fn apply(row: &impl IsA<gtk::Widget>, gap: i32, line_gap: i32) {
        let l: WrapRow = glib::Object::new();
        l.imp().gap.set(gap);
        l.imp().line_gap.set(line_gap);
        row.set_layout_manager(Some(l));
    }
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn one_line_when_it_fits() {
        let (at, h) = wrap(&[(50, 17), (60, 17), (40, 17)], 6, 6, 400);
        assert_eq!(at, vec![(0, 0), (56, 0), (122, 0)]);
        assert_eq!(h, 17);
    }

    #[test]
    fn wraps_to_a_new_line_keeping_each_width() {
        let (at, h) = wrap(&[(100, 17), (100, 17), (100, 17)], 6, 6, 220);
        assert_eq!(at, vec![(0, 0), (106, 0), (0, 23)]);
        assert_eq!(h, 40);
    }

    #[test]
    fn a_child_wider_than_the_line_has_a_line_of_its_own() {
        let (at, h) = wrap(&[(30, 17), (500, 17), (30, 17)], 6, 6, 200);
        assert_eq!(at, vec![(0, 0), (0, 23), (0, 46)]);
        assert_eq!(h, 63);
        assert_eq!(wrap(&[], 6, 6, 200), (vec![], 0));
    }
}
