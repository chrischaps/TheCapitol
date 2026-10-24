// The world HUD (plan §4.3): what hangs over the world, drawn on the
// screen. So far that's the players' names (P4.2), placed through
// world_to_screen, what they say over them (Commons S1.2), and the zone
// banner (P4.5).
//
// Rust's labels.rs projects where each name hangs with the camera this
// frame renders with, and decides which names to draw, where, how big and
// how faint (cap_name_tags); the script just draws them. Names fade and
// shrink with distance; nearer names win a clash, and a crowd shows only
// its nearest two dozen. Yours always comes first, in gold, and carries
// furthest. The placement moved to Rust in P8.5: in Rhai its overlap test
// was quadratic, and a crowd of 50 cost about 2 ms a frame. The projection
// followed in H14 (#97), once Rust could reach the frame's camera.

// The zone banner names the zone you've just walked into, in its colour,
// as an inscription: a small caption, the name in Cinzel capitals rising
// into place, and a rule that draws itself out from a diamond under it.
// The zone you come into the world in gets one too, once the camera has
// come down onto you (cap_zone_banner's age is negative until then).

// ── Tuning ───────────────────────────────────────────────────────────────

// The names' fades, sizes and spacing are in labels.rs, and so are the
// bubbles': the wrap, the padding and the tail's drop.

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
    let tags = cap_name_tags();
    // Every tail first, then the slips over them, so a tail running up
    // past a lower slip to a lifted one goes behind it.
    for t in tags {
        if t.bubble != () { bubble_tail(t.bubble); }
    }
    for t in tags {
        if t.bubble != () { bubble(t.bubble); }
    }
    for t in tags {
        // A speaker whose name had no room is named in their bubble.
        if !t.named { continue; }
        if t.glyph != () { citizen_glyph(t.glyph, t.a); }
        draw_text(t.x + 1.0, t.y + 1.0, t.name, t.size, 0.0, 0.0, 0.0, 0.65 * t.a);
        if t.me {
            // The Capitol's gold, #c9a227.
            draw_text(t.x, t.y, t.name, t.size, 0.788, 0.635, 0.153, t.a);
        } else {
            // The panels' body text, #e0e0e0.
            draw_text(t.x, t.y, t.name, t.size, 0.878, 0.878, 0.878, t.a);
        }
        // Their guild's tag (S4.4), shadowed, in the panels' dim ink, #808080.
        let g = t.tag;
        if g != () {
            draw_text(g.x + 1.0, g.y + 1.0, g.text, g.size, 0.0, 0.0, 0.0, 0.65 * t.a);
            draw_text(g.x, g.y, g.text, g.size, 0.502, 0.502, 0.502, t.a);
        }
    }
}

// What someone says (S1.2): a slip of the Gazette's paper over their
// head, its ink, a hairline edge and a tail down to the name. The
// citizen's lozenge stays on the name below. When the name had no room
// over the head, it heads the slip instead, smaller and in the dimmer ink,
// lozenge and all. The colours are the theme's (`paper`, `edge`, `ink`,
// `dim`), handed over by cap_name_tags.
// The tail: from under the slip down to the name (or the head), edged,
// with the paper running down inside it.
fn bubble_tail(b) {
    let a = b.a;
    let p = b.paper;
    let e = b.edge;
    let foot = b.y + b.h;
    let half = tail_half(b);
    let tail = [[b.tip_x - half, foot - 1.0], [b.tip_x + half, foot - 1.0], [b.tip_x, b.tip_y]];
    draw_polygon(tail, e[0], e[1], e[2], 0.9 * a, 0);
    let inner = [[b.tip_x - half + 1.0, foot - 1.0], [b.tip_x + half - 1.0, foot - 1.0], [b.tip_x, b.tip_y - 1.6]];
    draw_polygon(inner, p[0], p[1], p[2], 0.96 * a, 0);
}

// Half the tail's width where it leaves the slip.
fn tail_half(b) { (b.size * 0.32).max(1.5) }

fn bubble(b) {
    let a = b.a;
    let p = b.paper;
    let e = b.edge;
    let k = b.ink;
    let round = b.size * 0.45;
    let foot = b.y + b.h;
    // A soft shadow, so the slip lifts off a bright street.
    draw_rect_ex(b.x + 1.0, b.y + 2.0, b.w, b.h, 0.0, 0.0, 0.0, 0.28 * a, round, 0);
    draw_rect_ex(b.x, b.y, b.w, b.h, p[0], p[1], p[2], 0.96 * a, round, 0);
    draw_rect_outline_ex(b.x, b.y, b.w, b.h, e[0], e[1], e[2], 0.9 * a, 1.0, round, 0);
    // Open the edge where the tail leaves, so slip and tail are one.
    let half = tail_half(b) - 1.0;
    draw_rect(b.tip_x - half, foot - 2.0, 2.0 * half, 2.0, p[0], p[1], p[2], 0.96 * a);
    let c = b.caption;
    if c != () {
        let d = b.dim;
        if c.glyph != () {
            draw_polygon(lozenge(c.glyph.x, c.glyph.y, c.glyph.r), d[0], d[1], d[2], 0.9 * a, 0);
        }
        draw_text_opts(c.x, c.y, c.text, c.size, #{
            font: "strong",
            color: [d[0], d[1], d[2], a],
        });
    }
    for r in b.rows {
        draw_text_opts(r.x, r.y, r.text, b.size, #{
            align: "center",
            color: [k[0], k[1], k[2], a],
        });
    }
}

// A citizen's mark (C0.2): a slender lozenge, a lamp's pane, before the
// name, in their lantern's pale (#d6e4ea, entities.rs CITIZEN_LANTERN).
// Shadowed like the name, so it reads over a bright sky.
fn citizen_glyph(g, a) {
    // Layer 0, with the names.
    draw_polygon(lozenge(g.x + 1.0, g.y + 1.0, g.r), 0.0, 0.0, 0.0, 0.65 * a, 0);
    draw_polygon(lozenge(g.x, g.y, g.r), 0.839, 0.894, 0.918, 0.9 * a, 0);
}

fn lozenge(x, y, r) {
    let w = r * 0.72;
    [[x, y - r], [x + w, y], [x, y + r], [x - w, y]]
}

fn smooth(t) {
    let t = t.max(0.0).min(1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a, b, t) { a + (b - a) * t }
