// The P0.4 spike on master. The camera is the rig's (camera.rhai, P3.9).
// The ground, water and roads are runtime meshes the extension builds
// (P1.4); over them the moat and inner canal are traced as hairlines, the
// plan the city was drawn from. The inscription comes from Rust through
// cap_spike_note(), which CapitolExtension registers.
//
// Two inputs test the game egui layer (P1.3). P ("plan") shows and hides
// the plan; typing a p in the inscription box must not. A left click on
// the city leaves a gold ripple; a click on the box must not.
//
// The land stands cap_ground_y() metres above the sea (P3.3), and
// everything here is drawn on it.
//
// The cursor is picked off the ground with screen_to_ground (P1.5): a gold
// reticle lies on the ground under it, foreshortened by the view, and a
// click's ripple spreads across the ground like a stone dropped in still
// water, not across the glass of the screen.

let plan_shown = true;
let ripples = [];

// How long a ripple takes to spread and fade, in seconds. A function, not
// a top-level const, because helper functions can't see the script's scope.
fn ripple_life() { 1.6 }

fn on_update() {
    let t = total_time();
    if is_action_just_pressed("plan") {
        plan_shown = !plan_shown;
        log("plan " + if plan_shown { "shown" } else { "hidden" });
    }
    if is_mouse_just_pressed(0) {
        // The ground under the cursor, or () over the sky.
        let g = screen_to_ground(mouse_x(), mouse_y(), cap_ground_y());
        if g != () {
            ripples.push(#{ x: g[0], z: g[1], born: t });
            log("mark at " + g[0].round() + ", " + g[1].round() + " m");
        }
    }
    ripples = ripples.filter(|r| t - r.born < ripple_life());
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

// Two gold rings spreading over the ground from where the click landed,
// the second a beat behind, both easing out as they fade.
fn draw_ripple(rip, now) {
    for k in 0..2 {
        let age = (now - rip.born) / ripple_life() - k.to_float() * 0.2;
        if age <= 0.0 || age >= 1.0 { continue; }
        let ease = 1.0 - (1.0 - age) * (1.0 - age);
        let radius = 0.3 + 5.5 * ease;
        let alpha = 0.85 * (1.0 - age) * (1.0 - age);
        trace_ring_at(rip.x, rip.z, radius, [], 0.0, 0.788, 0.635, 0.153, alpha, 1.5, 48);
    }
}

// The reticle: a thin gold ring lying on the ground under the cursor, a
// fainter one outside it, and four ticks pointing north, east, south and
// west, so the ground's own perspective shows in its shape.
fn draw_reticle(g, now) {
    let breathe = 1.0 + 0.06 * (now * 2.4).sin();
    let r = 0.9 * breathe;
    trace_ring_at(g[0], g[1], r, [], 0.0, 0.788, 0.635, 0.153, 0.9, 1.5, 40);
    trace_ring_at(g[0], g[1], r * 1.9, [], 0.0, 0.788, 0.635, 0.153, 0.25, 1.0, 56);
    for a in [0.0, 0.5 * PI(), PI(), 1.5 * PI()] {
        let p = world_to_screen(g[0] + r * 1.25 * a.cos(), cap_ground_y(), g[1] + r * 1.25 * a.sin());
        let q = world_to_screen(g[0] + r * 1.7 * a.cos(), cap_ground_y(), g[1] + r * 1.7 * a.sin());
        if p.visible && q.visible {
            draw_line(p.x, p.y, q.x, q.y, 0.788, 0.635, 0.153, 0.7, 1.5);
        }
    }
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

    let now = total_time();
    for rip in ripples {
        draw_ripple(rip, now);
    }
    // Picked here rather than in on_update: on_draw_ui sees the camera this
    // frame renders with, so the reticle can't trail the ground under it.
    let hover = screen_to_ground(mouse_x(), mouse_y(), cap_ground_y());
    if hover != () {
        draw_reticle(hover, now);
    }
}
