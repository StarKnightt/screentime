# Vendored crates

`iced_graphics` 0.14.0 from crates.io (MIT, see its LICENSE), with one change in
`src/damage.rs`: when more than half of the window changed in a frame, it is
repainted once as a whole instead of region by region. Scrolling moves every
widget, and the software renderer redraws each large shape once per damaged
region through a clip mask, so a scrolling frame took about 20 ms instead of
about 3 ms. It is applied with `[patch.crates-io]` in the workspace Cargo.toml.
