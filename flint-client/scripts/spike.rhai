// The P0.4 spike on master. The camera is the rig's (camera.rhai, P3.9).
// The ground, water and roads are runtime meshes the extension builds
// (P1.4); over them the moat and inner canal are traced as hairlines, the
// plan the city was drawn from. The inscription comes from Rust through
// cap_spike_note(), which CapitolExtension registers.
//
// P ("plan") shows and hides the plan, which tests the game egui layer
// (P1.3): typing a p in the inscription box must not.
//
// The land stands cap_ground_y() metres above the sea (P3.3), and
// everything here is drawn on it. The click ripple and the cursor's
// reticle moved to input.rhai with click to move (P4.3).

let plan_shown = true;

fn on_update() {
    if is_action_just_pressed("plan") {
        plan_shown = !plan_shown;
        log("plan " + if plan_shown { "shown" } else { "hidden" });
    }
}

// A ring of radius r (metres) on the ground, with a gap of half-width
// `gap` (radians) at each angle in `gaps` (degrees, server convention:
// 0 = east, 90 = south). Drawn as projected segments.
fn trace_ring(r, gaps, gap, red, green, blue, alpha) {
    trace_ring_at(0.0, 0.0, r, gaps, gap, red, green, blue, alpha, 1.5, 96);
}

fn trace_ring_at(cx, cz, r, gaps, gap, red, green, blue, alpha, width, steps) {
    let prev = ();
    for i in 0..=steps {
        let a = i.to_float() / steps.to_float() * 2.0 * PI();
        let in_gap = false;
        for g in gaps {
            let d = (a - g.to_float().to_radians() + 3.0 * PI()) % (2.0 * PI()) - PI();
            if d.abs() < gap { in_gap = true; }
        }
        let p = world_to_screen(cx + r * a.cos(), cap_ground_y(), cz + r * a.sin());
        if in_gap || !p.visible {
            prev = ();
            continue;
        }
        if prev != () {
            draw_line(prev.x, prev.y, p.x, p.y, red, green, blue, alpha, width);
        }
        prev = p;
    }
}

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
    // Moat (server radius 200, one bridge to the south) and inner canal
    // (radius 600, four bridges), in the water blue of the 2D clients.
    if plan_shown {
        trace_ring(20.0, [90], 0.075, 0.30, 0.55, 0.85, 0.55);
        trace_ring(60.0, [0, 90, 180, 270], 0.021, 0.30, 0.55, 0.85, 0.35);
    }

    let note = cap_spike_note();
    if note != "" {
        label(0.0, cap_crown_y(), 0.0, note, 30.0, 0.788, 0.635, 0.153);
    }
}
