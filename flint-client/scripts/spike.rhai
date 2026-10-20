// The P0.4 spike on master. The camera is the rig's (camera.rhai, P3.9).
// The ground, water and roads are runtime meshes the extension builds
// (P1.4). The inscription comes from Rust through cap_spike_note(), which
// CapitolExtension registers.
//
// The moat and inner canal used to be traced over them as hairlines, the
// plan the city was drawn from, on P. They were drawn in screen space, over
// everything, and the living world (P4) draws the water itself, so they went
// in #142.
//
// The land stands cap_ground_y() metres above the sea (P3.3), and
// everything here is drawn on it. The click ripple and the cursor's
// reticle moved to input.rhai with click to move (P4.3).

// Text centred on the point (x, y, z), in metres, in the Capitol's
// inscriptional capitals (`display` in fonts/fonts.toml), spaced like the
// login form's title.
fn label(x, y, z, text, size, r, g, b) {
    let p = world_to_screen(x, y, z);
    if !p.visible { return; }
    let spacing = size * 0.18;
    let m = measure_text_ex(text, size, "display", spacing);
    draw_text_opts(p.x - m.width / 2.0, p.y - m.height / 2.0, text, size, #{
        font: "display",
        spacing: spacing,
        color: [r, g, b, 1.0],
        shadow: [1.5, 1.5, 0.0, 0.0, 0.0, 0.6],
    });
}

fn on_draw_ui() {
    let note = cap_spike_note();
    if note != "" {
        label(0.0, cap_crown_y(), 0.0, note, 30.0, 0.788, 0.635, 0.153);
    }
}
