// The world HUD (plan §4.3): what hangs over the world, drawn on the
// screen through world_to_screen. So far that's the players' names (P4.2).
//
// cap_players() lists every avatar Rust placed this frame (entities.rs):
// its name, where the name hangs over its head (world metres), how far it
// has faded in as the player joined or left, and whether it's you.
//
// Names fade with distance, and shrink a little, so a crowd down the
// street doesn't read as a wall of text. Nearer names win: one that would
// overlap a name already drawn is left out. Yours always comes first, in
// gold, and carries further than anyone else's, so you can find yourself
// from high up.

// ── Tuning ───────────────────────────────────────────────────────────────

// Names are whole out to `near` metres from the camera and gone by `far`.
fn near() { 70.0 }
fn far() { 170.0 }
// Yours carries further.
fn my_near() { 260.0 }
fn my_far() { 420.0 }
// Text size in points, close up and at the fade's end.
fn size_near() { 15.0 }
fn size_far() { 11.0 }
// Room kept between two names, in points.
fn gap() { 3.0 }

// ── Per frame ────────────────────────────────────────────────────────────

fn on_draw_ui() {
    if cap_at_login() { return; }
    let players = cap_players();
    if players.is_empty() { return; }

    // Each name on the screen, with what it's drawn at.
    let tags = [];
    for p in players {
        let s = world_to_screen(p.x, p.y, p.z);
        if !s.visible { continue; }
        let d = s.depth;
        let fade = if p.me {
            1.0 - smooth((d - my_near()) / (my_far() - my_near()))
        } else {
            1.0 - smooth((d - near()) / (far() - near()))
        };
        let a = fade * p.alpha;
        if a < 0.02 { continue; }
        let size = lerp(size_near(), size_far(), smooth((d - 20.0) / (far() - 20.0)));
        let m = measure_text(p.name, size);
        tags.push(#{
            name: p.name, me: p.me, a: a, size: size, depth: d,
            x: s.x - m.width / 2.0, y: s.y - m.height, w: m.width, h: m.height,
        });
    }
    // You first, then nearest first: those are the names that win a clash.
    tags.sort(|a, b| {
        if a.me != b.me { return if a.me { -1 } else { 1 }; }
        if a.depth < b.depth { -1 } else if a.depth > b.depth { 1 } else { 0 }
    });

    let drawn = [];
    for t in tags {
        let clear = true;
        for o in drawn {
            if t.x < o.x + o.w + gap() && o.x < t.x + t.w + gap()
                && t.y < o.y + o.h + gap() && o.y < t.y + t.h + gap() {
                clear = false;
                break;
            }
        }
        if !clear { continue; }
        drawn.push(t);
        draw_text(t.x + 1.0, t.y + 1.0, t.name, t.size, 0.0, 0.0, 0.0, 0.65 * t.a);
        if t.me {
            // The Capitol's gold, #c9a227.
            draw_text(t.x, t.y, t.name, t.size, 0.788, 0.635, 0.153, t.a);
        } else {
            // The panels' body text, #e0e0e0.
            draw_text(t.x, t.y, t.name, t.size, 0.878, 0.878, 0.878, t.a);
        }
    }
}

fn smooth(t) {
    let t = t.max(0.0).min(1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a, b, t) { a + (b - a) * t }
