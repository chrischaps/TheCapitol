// The P0.4 spike on master. Camera: south of the Capitol looking north, so
// east is on the right. Until the runtime meshes return (P1.4) the moat and
// inner canal are traced in the air as hairlines, a plan of the city drawn
// before it's built, and the inscription comes from Rust through
// cap_spike_note(), which CapitolExtension registers.

fn on_update() {
    // A slow breath of a drift, staying south of centre.
    let t = total_time();
    let sway = 6.0 * (t * 0.15).sin();
    set_camera_position(sway, 70.0, 95.0);
    set_camera_target(0.0, 0.0, 4.0);
}

// A ring of radius r (metres) on the ground, with a gap of half-width
// `gap` (radians) at each angle in `gaps` (degrees, server convention:
// 0 = east, 90 = south). Drawn as projected segments.
fn trace_ring(r, gaps, gap, red, green, blue, alpha) {
    let steps = 96;
    let prev = ();
    for i in 0..=steps {
        let a = i.to_float() / steps.to_float() * 2.0 * PI();
        let in_gap = false;
        for g in gaps {
            let d = (a - g.to_float().to_radians() + 3.0 * PI()) % (2.0 * PI()) - PI();
            if d.abs() < gap { in_gap = true; }
        }
        let p = world_to_screen(r * a.cos(), 0.0, r * a.sin());
        if in_gap || !p.visible {
            prev = ();
            continue;
        }
        if prev != () {
            draw_line(prev.x, prev.y, p.x, p.y, red, green, blue, alpha, 1.5);
        }
        prev = p;
    }
}

fn label(x, z, text, size, r, g, b) {
    let p = world_to_screen(x, 0.5, z);
    if !p.visible { return; }
    let m = measure_text(text, size);
    let tx = p.x - m.width / 2.0;
    let ty = p.y - m.height / 2.0;
    draw_text(tx + 1.5, ty + 1.5, text, size, 0.0, 0.0, 0.0, 0.6);
    draw_text(tx, ty, text, size, r, g, b, 1.0);
}

fn on_draw_ui() {
    // Moat (server radius 200, one bridge to the south) and inner canal
    // (radius 600, four bridges), in the water blue of the 2D clients.
    trace_ring(20.0, [90], 0.075, 0.30, 0.55, 0.85, 0.55);
    trace_ring(60.0, [0, 90, 180, 270], 0.021, 0.30, 0.55, 0.85, 0.35);

    let note = cap_spike_note();
    if note != "" {
        label(0.0, 0.0, note, 30.0, 0.788, 0.635, 0.153);
    }
}
