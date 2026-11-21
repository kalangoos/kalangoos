//! A row that shows only what fits: its children side by side at their natural width, and the ones that don't fit
//! hidden (not cut). The first and the last child always stay (the system's agent and "Todos os agentes" in the chat's
//! start), and so does a child marked `ativo` (the agent chosen); the others come in order while there is room.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// Which children show in `avail` px: `widths` are their natural widths, `gap` the space between two; the first, the
/// last and `keep` come first, then the others in order while they fit.
pub fn fitting(widths: &[i32], gap: i32, avail: i32, keep: Option<usize>) -> Vec<bool> {
    let n = widths.len();
    let mut shown = vec![false; n];
    if n == 0 {
        return shown;
    }
    let mut used = -gap;
    let mut order: Vec<usize> = vec![0, n - 1];
    if let Some(k) = keep.filter(|k| *k < n) {
        order.push(k);
    }
    let required = order.len();
    order.extend(1..n.saturating_sub(1));
    for (i, k) in order.into_iter().enumerate() {
        if shown[k] {
            continue;
        }
        let w = used + gap + widths[k];
        // (the required ones show even without room: they shrink)
        if i < required || w <= avail {
            shown[k] = true;
            used = w;
        }
    }
    shown
}

mod imp {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    pub struct FitRow {
        pub gap: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FitRow {
        const NAME: &'static str = "KalangoFitRow";
        type Type = super::FitRow;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for FitRow {}

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

    impl LayoutManagerImpl for FitRow {
        fn request_mode(&self, _: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, widget: &gtk::Widget, o: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let kids = children(widget);
            let gap = self.gap.get();
            if o == gtk::Orientation::Vertical {
                let (min, nat) = kids.iter().map(|c| c.measure(o, -1)).fold((0, 0), |(a, b), (m, n, _, _)| (a.max(m), b.max(n)));
                return (min, nat, -1, -1);
            }
            // the least: the first and the last; the natural: everything
            let sizes: Vec<(i32, i32)> = kids.iter().map(|c| {
                let (m, n, _, _) = c.measure(o, -1);
                (m, n)
            }).collect();
            let nat = sizes.iter().map(|s| s.1).sum::<i32>() + gap * (sizes.len() as i32 - 1).max(0);
            let min = match sizes.as_slice() {
                [] => 0,
                [one] => one.1,
                [first, .., last] => first.1 + gap + last.1,
            };
            (min, nat.max(min), -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _baseline: i32) {
            let kids = children(widget);
            let gap = self.gap.get();
            let sizes: Vec<(i32, i32)> = kids.iter().map(|c| {
                let (m, n, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
                (m, n)
            }).collect();
            let keep = kids.iter().position(|c| c.has_css_class("ativo"));
            let shown = fitting(&sizes.iter().map(|s| s.1).collect::<Vec<_>>(), gap, width, keep);
            let mut widths: Vec<i32> = sizes.iter().zip(&shown).map(|(s, on)| if *on { s.1 } else { 0 }).collect();
            let count = shown.iter().filter(|s| **s).count() as i32;
            let mut over = widths.iter().sum::<i32>() + gap * (count - 1).max(0) - width;
            // (only when even the required ones don't fit: they give way down to their least, the first ones first)
            for (i, w) in widths.iter_mut().enumerate() {
                if over <= 0 {
                    break;
                }
                if shown[i] {
                    let give = (*w - sizes[i].0).min(over).max(0);
                    *w -= give;
                    over -= give;
                }
            }
            let total = widths.iter().sum::<i32>() + gap * (count - 1).max(0);
            let mut x = ((width - total) / 2).max(0);
            for (i, c) in kids.iter().enumerate() {
                c.set_child_visible(shown[i]);
                if shown[i] {
                    c.size_allocate(&gtk::Allocation::new(x, 0, widths[i], height), -1);
                    x += widths[i] + gap;
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct FitRow(ObjectSubclass<imp::FitRow>) @extends gtk::LayoutManager;
}

impl FitRow {
    /// Lays `row`'s children out in a line that shows only what fits, `gap` px apart.
    pub fn apply(row: &impl IsA<gtk::Widget>, gap: i32) {
        let l: FitRow = glib::Object::new();
        l.imp().gap.set(gap);
        row.set_layout_manager(Some(l));
    }
}

#[cfg(test)]
mod tests {
    use super::fitting;

    #[test]
    fn everything_shows_when_it_fits() {
        assert_eq!(fitting(&[100, 110, 140, 130, 120], 8, 700, None), vec![true; 5]);
    }

    #[test]
    fn the_ones_that_dont_fit_hide_and_the_ends_stay() {
        // 100 + 120 = 220 (+8), then 110 (338), 140 would make 486 > 400
        assert_eq!(fitting(&[100, 110, 140, 130, 120], 8, 400, None), vec![true, true, false, false, true]);
        // a smaller one later still comes in
        assert_eq!(fitting(&[100, 110, 140, 40, 120], 8, 400, None), vec![true, true, false, true, true]);
        // without room the ends still show (they shrink)
        assert_eq!(fitting(&[100, 110, 120], 8, 150, None), vec![true, false, true]);
    }

    #[test]
    fn the_chosen_one_stays() {
        assert_eq!(fitting(&[100, 110, 140, 130, 120], 8, 400, Some(3)), vec![true, false, false, true, true]);
        assert_eq!(fitting(&[], 8, 400, Some(3)), Vec::<bool>::new());
        assert_eq!(fitting(&[90], 8, 10, None), vec![true]);
    }
}
