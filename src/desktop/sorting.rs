use gtk::prelude::*;
use gtk::{cairo, glib};

pub(super) struct SortField<'a> {
    pub label: &'a str,
    /// Action target and human-readable direction, in ascending / descending order.
    pub directions: [(&'a str, &'a str); 2],
}

#[derive(Clone, Copy)]
enum Glyph {
    Sort,
    Ascending,
    Descending,
}

fn triangle(context: &cairo::Context, x: f64, y: f64, ascending: bool) {
    let sign = if ascending { -1.0 } else { 1.0 };
    context.move_to(x, y + sign * 2.5);
    context.line_to(x + 3.5, y - sign * 2.0);
    context.line_to(x - 3.5, y - sign * 2.0);
    context.close_path();
}

// Small, theme-colored vectors on a 16px grid; no font glyphs or theme-dependent
// sort icons. The paired triangles repeat the same language as the menu controls.
fn icon(glyph: Glyph) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(16)
        .content_height(16)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .can_target(false)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    area.set_draw_func(move |area, context, width, height| {
        let size = f64::from(width.min(height));
        if size <= 0.0 || context.save().is_err() {
            return;
        }
        context.translate(
            (f64::from(width) - size) / 2.0,
            (f64::from(height) - size) / 2.0,
        );
        context.scale(size / 16.0, size / 16.0);
        let color = area.style_context().color();
        context.set_source_rgba(
            color.red().into(),
            color.green().into(),
            color.blue().into(),
            color.alpha().into(),
        );
        context.new_path();
        match glyph {
            Glyph::Sort => {
                context.set_line_width(1.5);
                context.set_line_cap(cairo::LineCap::Round);
                for (y, end) in [(4.0, 8.0), (8.0, 6.0), (12.0, 4.0)] {
                    context.move_to(1.5, y);
                    context.line_to(end, y);
                }
                let _ = context.stroke();
                context.translate(12.5, 8.0);
                context.scale(0.72, 0.72);
                triangle(context, 0.0, -4.0, true);
                triangle(context, 0.0, 4.0, false);
            }
            Glyph::Ascending => triangle(context, 8.0, 8.0, true),
            Glyph::Descending => triangle(context, 8.0, 8.0, false),
        }
        let _ = context.fill();
        let _ = context.restore();
    });
    area
}

pub(super) fn sort_button(action: &str, label: &str, fields: &[SortField<'_>]) -> gtk::MenuButton {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.add_css_class("sort-options");
    for field in fields {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        row.add_css_class("sort-field");
        let name = gtk::Label::builder()
            .label(field.label)
            .xalign(0.0)
            .hexpand(true)
            .margin_end(16)
            .build();
        row.append(&name);
        for ((target, direction), glyph) in field
            .directions
            .iter()
            .zip([Glyph::Ascending, Glyph::Descending])
        {
            let description = format!("{} · {direction}", field.label);
            let button = gtk::ToggleButton::builder()
                .child(&icon(glyph))
                .tooltip_text(&description)
                .build();
            button.add_css_class("flat");
            button.add_css_class("sort-direction");
            button.update_property(&[gtk::accessible::Property::Label(&description)]);
            // The existing string-valued action is the only source of selection
            // state. It also restores the right triangle when changing Folders.
            button.set_action_target_value(Some(&target.to_variant()));
            button.set_action_name(Some(action));
            row.append(&button);
        }
        content.append(&row);
    }
    let popover = gtk::Popover::builder().child(&content).build();
    let button = gtk::MenuButton::builder()
        .child(&icon(Glyph::Sort))
        .tooltip_text(label)
        .popover(&popover)
        .build();
    button.add_css_class("flat");
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    // Opening with either pointer or keyboard starts on the current selection.
    popover.connect_map(glib::clone!(
        #[weak]
        content,
        move |_| {
            let mut row = content.first_child();
            while let Some(field) = row {
                let mut child = field.first_child();
                while let Some(widget) = child {
                    if let Some(button) = widget.downcast_ref::<gtk::ToggleButton>()
                        && button.is_active()
                    {
                        button.grab_focus();
                        return;
                    }
                    child = widget.next_sibling();
                }
                row = field.next_sibling();
            }
        }
    ));
    button
}
