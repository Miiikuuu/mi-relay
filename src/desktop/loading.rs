use std::cell::{Cell, RefCell};
use std::f64::consts::TAU;
use std::rc::Rc;
use std::sync::OnceLock;

use gtk::prelude::*;
use gtk::{cairo, glib};

// SVG user units from the supplied DashRing (viewBox="0 0 24 24").
const VIEWBOX_SIZE: f64 = 24.0;
const RADIUS: f64 = 9.5;
const STROKE_WIDTH: f64 = 2.0;
const TRACK_OPACITY: f64 = 0.1;
const DASH_GAP: f64 = 150.0;
const REVOLUTION_MICROSECONDS: i64 = 2_000_000;
const DASH_MICROSECONDS: i64 = 1_500_000;
const STATIC_FRAME_MICROSECONDS: i64 = 750_000;

// Shared frame-clock origin avoids restarting the dash each time a transfer row
// is rebuilt. Both independent animation periods still start at the same origin.
static CLOCK_ORIGIN: OnceLock<i64> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq)]
struct DashFrame {
    rotation: f64,
    length: f64,
    offset: f64,
}

impl DashFrame {
    fn at(elapsed: i64) -> Self {
        let turn =
            elapsed.rem_euclid(REVOLUTION_MICROSECONDS) as f64 / REVOLUTION_MICROSECONDS as f64;
        let dash = elapsed.rem_euclid(DASH_MICROSECONDS) as f64 / DASH_MICROSECONDS as f64;
        let (length, offset) = if dash < 0.5 {
            // SMIL defaults to linear interpolation, with keyTimes="0;0.5;1".
            (42.0 * dash * 2.0, -16.0 * dash * 2.0)
        } else {
            (42.0, -16.0 - 43.0 * (dash - 0.5) * 2.0)
        };
        Self {
            rotation: turn * TAU,
            length,
            offset,
        }
    }
}

#[derive(Default)]
struct Animation {
    elapsed: Cell<i64>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
}

impl Animation {
    fn stop(&self) {
        let tick = self.tick.borrow_mut().take();
        if let Some(tick) = tick {
            tick.remove();
        }
    }
}

fn update_animation(area: &gtk::DrawingArea, animation: &Rc<Animation>) {
    let enabled = area.settings().is_gtk_enable_animations();
    if area.is_mapped() && enabled {
        if animation.tick.borrow().is_none() {
            let state = Rc::clone(animation);
            let tick = area.add_tick_callback(move |area, clock| {
                if !area.is_mapped() || !area.settings().is_gtk_enable_animations() {
                    // Returning Break removes this callback; do not remove it twice.
                    state.tick.borrow_mut().take();
                    return glib::ControlFlow::Break;
                }
                let now = clock.frame_time();
                let origin = *CLOCK_ORIGIN.get_or_init(|| now);
                state.elapsed.set(now.saturating_sub(origin).max(0));
                area.queue_draw();
                glib::ControlFlow::Continue
            });
            *animation.tick.borrow_mut() = Some(tick);
        }
    } else {
        animation.stop();
        if !enabled {
            // Freeze on a visible arc, not the zero-length starting dash.
            animation.elapsed.set(STATIC_FRAME_MICROSECONDS);
        }
    }
    if area.is_mapped() {
        area.queue_draw();
    }
}

fn draw_ring(
    context: &cairo::Context,
    width: i32,
    height: i32,
    color: [f64; 4],
    frame: DashFrame,
) -> Result<(), cairo::Error> {
    if width <= 0 || height <= 0 {
        return Ok(());
    }
    context.save()?;
    let result = (|| {
        let scale = f64::from(width.min(height)) / VIEWBOX_SIZE;
        context.translate(f64::from(width) / 2.0, f64::from(height) / 2.0);
        context.scale(scale, scale);
        context.set_line_width(STROKE_WIDTH);
        context.set_line_cap(cairo::LineCap::Round);
        context.set_dash(&[], 0.0);
        context.set_source_rgba(color[0], color[1], color[2], color[3] * TRACK_OPACITY);
        context.new_path();
        context.arc(0.0, 0.0, RADIUS, 0.0, TAU);
        context.close_path();
        context.stroke()?;

        context.set_source_rgba(color[0], color[1], color[2], color[3]);
        context.rotate(frame.rotation);
        // SVG circles start at 3 o'clock, clockwise. Preserve the actual dash
        // and its signed offset: the arc contracts by clipping at the seam.
        // A zero-length dash with round caps is a dot, not an empty stroke.
        context.set_dash(&[frame.length, DASH_GAP], frame.offset);
        context.arc(0.0, 0.0, RADIUS, 0.0, TAU);
        context.close_path();
        context.stroke()
    })();
    let restored = context.restore();
    result.and(restored)
}

// Native rendering of the supplied SVG DashRing, shared by header and file rows.
pub(super) fn loading_ring(size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(size.max(1))
        .content_height(size.max(1))
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .can_target(false)
        .build();
    let animation = Rc::new(Animation::default());
    let state = Rc::clone(&animation);
    area.set_draw_func(move |area, context, width, height| {
        // StyleContext keeps compatibility with the project's baseline GTK features.
        let color = area.style_context().color();
        let _ = draw_ring(
            context,
            width,
            height,
            [
                f64::from(color.red()),
                f64::from(color.green()),
                f64::from(color.blue()),
                f64::from(color.alpha()),
            ],
            DashFrame::at(state.elapsed.get()),
        );
    });

    let state = Rc::clone(&animation);
    area.connect_map(move |area| update_animation(area, &state));
    let state = Rc::clone(&animation);
    area.connect_unmap(move |_| state.stop());

    // The watched widget invalidates this settings connection on destruction.
    // Neither this closure nor the frame callback owns a strong widget reference.
    area.settings().connect_closure(
        "notify::gtk-enable-animations",
        false,
        glib::closure_local!(
            #[watch]
            area,
            #[strong]
            animation,
            move |_settings: gtk::Settings, _property: glib::ParamSpec| {
                update_animation(area, &animation);
            }
        ),
    );
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_is_finite_and_bounded_even_for_extreme_frame_times() {
        for time in [i64::MIN, -1, 0, 1, 749_999, 750_000, 1_499_999, i64::MAX] {
            let frame = DashFrame::at(time);
            assert!(
                frame.rotation.is_finite() && frame.length.is_finite() && frame.offset.is_finite()
            );
            assert!((0.0..TAU).contains(&frame.rotation));
            assert!((0.0..=42.0).contains(&frame.length));
            assert!((-59.0..=0.0).contains(&frame.offset));
        }
    }

    #[test]
    fn svg_keyframes_and_linear_midpoints_match() {
        for (time, length, offset) in [
            (0, 0.0, 0.0),
            (375_000, 21.0, -8.0),
            (750_000, 42.0, -16.0),
            (1_125_000, 42.0, -37.5),
            (1_500_000, 0.0, 0.0),
        ] {
            let frame = DashFrame::at(time);
            assert_eq!((frame.length, frame.offset), (length, offset));
            assert!((frame.rotation - time as f64 / 2_000_000.0 * TAU).abs() < 1e-12);
        }
        assert!((DashFrame::at(1_499_999).offset + 59.0).abs() < 0.0001);
    }

    #[test]
    fn rotation_and_dash_have_independent_periods_and_repeat_together_at_six_seconds() {
        let start = DashFrame::at(0);
        assert_eq!(DashFrame::at(2_000_000).rotation, start.rotation);
        assert_ne!(DashFrame::at(2_000_000).length, start.length);
        assert_ne!(DashFrame::at(1_500_000).rotation, start.rotation);
        for time in [0, 123, 375_000, 750_000, 1_125_000, 5_999_999] {
            assert_eq!(DashFrame::at(time), DashFrame::at(time + 6_000_000));
        }
    }

    fn render(time: i64, width: i32, height: i32, color: [f64; 4]) -> cairo::ImageSurface {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height).unwrap();
        let context = cairo::Context::new(&surface).unwrap();
        draw_ring(&context, width, height, color, DashFrame::at(time)).unwrap();
        // The drawing helper must leave its caller's transform and style intact.
        assert_eq!(context.matrix(), cairo::Matrix::identity());
        assert_eq!(context.dash_count(), 0);
        surface
    }

    fn alpha_at(surface: &mut cairo::ImageSurface, x: usize, y: usize) -> u8 {
        let offset = y * surface.stride() as usize + x * 4;
        let data = surface.data().unwrap();
        (u32::from_ne_bytes(data[offset..offset + 4].try_into().unwrap()) >> 24) as u8
    }

    #[test]
    fn track_has_ten_percent_opacity_and_zero_dash_is_a_round_dot() {
        let mut surface = render(0, 240, 240, [1.0; 4]);
        assert!((25..=26).contains(&alpha_at(&mut surface, 120, 25)));
        assert_eq!(alpha_at(&mut surface, 215, 120), 255);
        assert_eq!(alpha_at(&mut surface, 0, 0), 0);
    }

    #[test]
    fn foreground_clips_at_the_seam_instead_of_wrapping_the_full_dash() {
        let mut surface = render(1_499_999, 240, 240, [1.0; 4]);
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let opaque = (0..240)
            .flat_map(|y| (0..240).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let offset = y * stride + x * 4;
                u32::from_ne_bytes(data[offset..offset + 4].try_into().unwrap()) >> 24 > 128
            })
            .count();
        assert!(
            (100..600).contains(&opaque),
            "expected a short round-ended remnant, got {opaque} opaque pixels"
        );
    }

    #[test]
    fn viewbox_is_centered_and_scaled_uniformly() {
        let mut surface = render(0, 240, 120, [1.0; 4]);
        assert_eq!(alpha_at(&mut surface, 168, 60), 255);
        assert!((25..=26).contains(&alpha_at(&mut surface, 120, 12)));
        assert_eq!(alpha_at(&mut surface, 215, 60), 0);
        for size in [1, 16, 24, 32] {
            render(750_000, size, size, [1.0; 4]);
        }
    }

    #[test]
    fn reduced_motion_frame_remains_visible_and_theme_alpha_is_respected() {
        assert_eq!(DashFrame::at(STATIC_FRAME_MICROSECONDS).length, 42.0);
        let mut surface = render(STATIC_FRAME_MICROSECONDS, 240, 240, [1.0, 1.0, 1.0, 0.5]);
        let data = surface.data().unwrap();
        let max_alpha = data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| u32::from_ne_bytes(*pixel) >> 24)
            .max()
            .unwrap();
        assert!((127..=141).contains(&max_alpha));
    }
}
