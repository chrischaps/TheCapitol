// The world HUD (plan §4.3): what hangs over the world, drawn on the
// screen. So far that's the players' names (P4.2), placed through
// world_to_screen, and the zone banner (P4.5).
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

// The zone banner names the zone you've just walked into, in its colour,
// as an inscription: a small caption, the name in Cinzel capitals rising
// into place, and a rule that draws itself out from a diamond under it.
// The zone you come into the world in gets one too, once the camera has
// come down onto you (cap_zone_banner's age is negative until then).

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

// The banner: seconds to fade in, when it starts to fade out, and gone.
fn banner_in() { 0.8 }
fn banner_hold() { 3.4 }
fn banner_out() { 4.6 }
// Its height on the screen, as a fraction from the top.
fn banner_y() { 0.2 }
fn banner_size() { 38.0 }
// How far the name's colour is lifted toward warm white.
fn name_lift() { 0.35 }

// ── Per frame ────────────────────────────────────────────────────────────

fn on_draw_ui() {
    if cap_at_login() { return; }
    zone_banner();
    names();
}

// ── The zone banner ──────────────────────────────────────────────────────

fn zone_banner() {
    let b = cap_zone_banner();
    if b == () { return; }
    let t = b.age;
    if t < 0.0 || t > banner_out() { return; }
    let a = smooth(t / banner_in()) * (1.0 - smooth((t - banner_hold()) / (banner_out() - banner_hold())));
    if a < 0.01 { return; }

    let cx = screen_width() / 2.0;
    let size = banner_size();
    // The name rises a little into place as it fades in.
    let y = screen_height() * banner_y() + (1.0 - smooth(t / 1.2)) * 8.0;
    // The name is the zone's colour lifted toward the lantern-light, so a
    // dark one (Urban's slate) still reads at dusk; the rule keeps it pure.
    let lift = name_lift();
    let colour = [lerp(b.r, 0.96, lift), lerp(b.g, 0.93, lift), lerp(b.b, 0.86, lift), a];

    let caption = if b.first { "YOU ARRIVE IN" } else { "ENTERING" };
    draw_text_opts(cx, y - 24.0, caption, 12.0, #{
        font: "strong", align: "center", spacing: 5.0,
        color: [0.878, 0.878, 0.878, 0.7 * a],
        shadow: [0.0, 1.0, 0.0, 0.0, 0.0, 0.6 * a],
    });
    draw_text_opts(cx, y, b.name, size, #{
        font: "display", align: "center", spacing: 4.0,
        color: colour,
        shadow: [0.0, 2.0, 0.0, 0.0, 0.0, 0.7 * a],
    });

    // The rule, drawn out from the centre under the name, fading at its ends.
    let m = measure_text_ex(b.name, size, "display", 4.0);
    let ry = y + m.height + 10.0;
    let reach = (m.width / 2.0 + 70.0) * smooth((t - 0.15) / 1.1);
    if reach > 1.0 {
        draw_rect_gradient(cx - reach, ry, reach, 1.5,
            b.r, b.g, b.b, 0.0, b.r, b.g, b.b, 0.9 * a, false, 10);
        draw_rect_gradient(cx, ry, reach, 1.5,
            b.r, b.g, b.b, 0.9 * a, b.r, b.g, b.b, 0.0, false, 10);
    }
    let d = 4.0;
    draw_polygon([[cx, ry + 0.75 - d], [cx + d, ry + 0.75], [cx, ry + 0.75 + d], [cx - d, ry + 0.75]],
        b.r, b.g, b.b, a, 11);
}

// ── The names ────────────────────────────────────────────────────────────

fn names() {
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
