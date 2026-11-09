//! Smooth wheel scrolling for iced's `scrollable`.
//!
//! iced moves a scrollable 60 px per wheel line in one jump. This wrapper
//! catches whole-line wheel ticks (a notched mouse wheel) and eases the offset
//! to its target over `DURATION` instead; a tick that lands mid-animation
//! retargets from where the content is now, like a browser. Fractional deltas
//! (precision touchpads, free-spinning wheels) are already fine-grained, so they
//! pass straight through, and so do scrollbar drags; both cancel an animation.
//!
//! It drives itself with `request_redraw` only while an animation is running,
//! so an idle window draws nothing extra and needs no subscription.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, Scrollable};
use iced::advanced::widget::{tree, Id, Operation, Tree};
use iced::advanced::{overlay, renderer, Clipboard, Shell, Widget};
use iced::{mouse, window, Element, Event, Length, Rectangle, Size, Vector};

/// How far one wheel line moves the content (iced's own step).
const LINE: f32 = 60.0;
/// How long a wheel step takes to settle.
const DURATION: Duration = Duration::from_millis(150);

pub struct Smooth<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    enabled: bool,
}

/// Wrap a scrollable. With `enabled` false (reduced motion) the wheel jumps as before.
pub fn smooth<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    enabled: bool,
) -> Smooth<'a, Message, Theme, Renderer> {
    Smooth { content: content.into(), enabled }
}

#[derive(Default)]
struct State {
    anim: Option<Anim>,
}

#[derive(Clone, Copy)]
struct Anim {
    from: f32,
    to: f32,
    start: Instant,
}

impl Anim {
    fn at(&self, now: Instant) -> (f32, bool) {
        let t = (now.saturating_duration_since(self.start).as_secs_f32() / DURATION.as_secs_f32()).min(1.0);
        let e = 1.0 - (1.0 - t).powi(3); // ease out cubic
        (self.from + (self.to - self.from) * e, t >= 1.0)
    }
}

/// Reads the outermost scrollable's offset and range, and optionally sets its
/// vertical offset.
struct Probe {
    set: Option<f32>,
    seen: Option<(f32, f32)>,
}

impl Operation for Probe {
    fn traverse(&mut self, _operate: &mut dyn FnMut(&mut dyn Operation)) {}

    fn scrollable(&mut self, _id: Option<&Id>, bounds: Rectangle, content: Rectangle, translation: Vector, state: &mut dyn Scrollable) {
        if self.seen.is_some() {
            return;
        }
        let max = (content.height - bounds.height).max(0.0);
        self.seen = Some((translation.y, max));
        if let Some(y) = self.set {
            state.scroll_to(AbsoluteOffset { x: None, y: Some(y.clamp(0.0, max)) });
        }
    }
}

impl<Message, Theme, Renderer> Smooth<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn probe(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, set: Option<f32>) -> Option<(f32, f32)> {
        let mut op = Probe { set, seen: None };
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, &mut op);
        op.seen
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Smooth<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        match event {
            // A notched wheel: animate instead of jumping.
            Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Lines { x, y } })
                if self.enabled && *x == 0.0 && *y != 0.0 && y.fract() == 0.0 && cursor.is_over(layout.bounds()) =>
            {
                if let Some((now_y, max)) = self.probe(tree, layout, renderer, None) {
                    let st = tree.state.downcast_mut::<State>();
                    let (from, base) = match st.anim {
                        Some(a) => (a.at(Instant::now()).0, a.to),
                        None => (now_y, now_y),
                    };
                    let to = (base - y * LINE).clamp(0.0, max);
                    if (to - from).abs() > 0.5 {
                        st.anim = Some(Anim { from, to, start: Instant::now() });
                        shell.request_redraw();
                    }
                    shell.capture_event();
                    return;
                }
            }
            // Touchpads and other fine deltas, and scrollbar grabs, take over at once.
            Event::Mouse(mouse::Event::WheelScrolled { .. }) | Event::Mouse(mouse::Event::ButtonPressed(_)) => {
                tree.state.downcast_mut::<State>().anim = None;
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let Some(a) = tree.state.downcast_ref::<State>().anim {
                    let (y, done) = a.at(*now);
                    self.probe(tree, layout, renderer, Some(y));
                    if done {
                        tree.state.downcast_mut::<State>().anim = None;
                    } else {
                        shell.request_redraw();
                    }
                }
            }
            _ => {}
        }
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a, Theme: 'a, Renderer> From<Smooth<'a, Message, Theme, Renderer>> for Element<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer + 'a,
{
    fn from(s: Smooth<'a, Message, Theme, Renderer>) -> Self {
        Element::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_eases_out_and_lands_exactly() {
        let start = Instant::now();
        let a = Anim { from: 0.0, to: 60.0, start };
        let (y0, d0) = a.at(start);
        let (y_mid, d_mid) = a.at(start + DURATION / 2);
        let (y1, d1) = a.at(start + DURATION);
        assert_eq!((y0, d0), (0.0, false));
        assert!(y_mid > 30.0 && y_mid < 60.0 && !d_mid, "ease-out is past halfway at half time: {y_mid}");
        assert_eq!((y1, d1), (60.0, true));
        assert_eq!(a.at(start + DURATION * 3), (60.0, true));
    }
}
