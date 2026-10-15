// Input to intents (plan §4.2): what a left click on the world means, and
// the marks it leaves. P4.3 is click to move.
//
// A click is picked off the ground with screen_to_ground and offered to
// each intent in the React client's order (GameCanvas.tsx handleClick).
// The first that takes it wins:
//
//   1. placing a station: put it here              (P6.2)
//   2. shift-click a player: trade, or walk closer  (P7.3)
//   3. a station: open it, or walk closer           (P6.3)
//   4. alt-click a plot: select it                  (P6.4)
//   5. an available resource node: extract          (P4.4)
//   6. anywhere else: walk there                    (P4.3, here)
//
// Each of 1-5 is a function below that says no for now. To add one, fill
// its body: it gets the click (#{ x, z } in world metres, `shift`, `alt`)
// and returns true when it took it.
//
// Walking: cap_move(x, z) sends you (clamped to the world, 0-8000 server
// units), and cap_destination() is where you're going until you arrive.
// A gold ring lies there, with a faint dashed line marching to it from
// your feet. The click itself drops a ripple, as the spike's did (P0.4).
//
// The server stops a walk at a canal's edge with `movement_blocked`. Rust
// fires it here with where you stand and which way you were going. The
// water splashes just ahead of you, the camera rig gives a small nudge
// (camera.rhai), and a toast says to find a bridge.
//
// The right button is the camera's (a right-drag orbits), so this is the
// left button only. egui keeps any click over its panels.

let ripples = [];
// The ring at the destination: #{ x, z, seen } while it's there, then
// `gone` from when you arrived, as it fades.
let marker = ();
let splashes = [];
let toast = ();

// ── Tuning ───────────────────────────────────────────────────────────────

// How long a click's ripple takes to spread and fade, in seconds.
fn ripple_life() { 1.6 }
// How long the destination ring takes to close once you arrive.
fn marker_fade() { 0.45 }
// The splash: how long, and how far past where you stopped (m).
fn splash_life() { 1.1 }
fn splash_ahead() { 1.1 }
// The toast: in, held, out (s).
fn toast_in() { 0.25 }
fn toast_hold() { 2.8 }
fn toast_out() { 0.7 }

// ── Per frame ────────────────────────────────────────────────────────────

fn on_update() {
    let t = total_time();
    ripples = ripples.filter(|r| t - r.born < ripple_life());
    splashes = splashes.filter(|s| t - s.born < splash_life());
    if toast != () && t - toast.born > toast_in() + toast_hold() + toast_out() {
        toast = ();
    }
    if cap_at_login() || !is_mouse_just_pressed(0) { return; }

    // The ground under the cursor, or () over the sky.
    let g = screen_to_ground(mouse_x(), mouse_y(), cap_ground_y());
    if g == () { return; }
    let click = #{
        x: g[0], z: g[1],
        shift: is_action_pressed("with_shift"),
        alt: is_action_pressed("with_alt"),
    };
    if place_station(click) { return; }
    if trade_with(click) { return; }
    if open_station(click) { return; }
    if select_plot(click) { return; }
    if extract(click) { return; }
    cap_move(click.x, click.z);
    ripples.push(#{ x: click.x, z: click.z, born: t });
}

// ── The intents above walking, waiting for their issues ──────────────────

fn place_station(click) { false } // P6.2
fn trade_with(click) { false }    // P7.3, shift
fn open_station(click) { false }  // P6.3
fn select_plot(click) { false }   // P6.4, alt
fn extract(click) { false }       // P4.4

// ── Server happenings ────────────────────────────────────────────────────

fn on_event(name, data) {
    if name != "movement_blocked" { return; }
    let t = total_time();
    let x = data.x + data.dir_x * splash_ahead();
    let z = data.z + data.dir_z * splash_ahead();
    // Droplets thrown up and on over the water, fixed at the splash.
    let drops = [];
    let heading = if data.dir_x == 0.0 && data.dir_z == 0.0 { random() * 2.0 * PI() }
                  else { atan(data.dir_z, data.dir_x) };
    for i in 0..14 {
        let a = heading + random_range(-1.3, 1.3);
        let out = random_range(0.4, 1.4);
        drops.push(#{
            vx: a.cos() * out, vz: a.sin() * out,
            vy: random_range(2.0, 3.8),
            size: random_range(1.6, 3.2),
        });
    }
    splashes.push(#{ x: x, z: z, born: t, drops: drops });
    // A walk stopped by water has no destination left to mark.
    if marker != () && marker.gone == () { marker.gone = t; }
    let water = data.water;
    let body = water.sub_string(0, 1).to_upper() + water.sub_string(1) + " has no crossing here";
    toast = #{ title: "Find a bridge", body: body, born: t };
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn on_draw_ui() {
    if cap_at_login() { return; }
    let now = total_time();

    let d = cap_destination();
    if d != () {
        if marker == () || marker.gone != () || marker.x != d[0] || marker.z != d[1] {
            marker = #{ x: d[0], z: d[1], seen: now, gone: () };
        }
    } else if marker != () && marker.gone == () {
        marker.gone = now;
    }
    if marker != () && marker.gone != () && now - marker.gone > marker_fade() {
        marker = ();
    }
    if marker != () {
        let you = cap_follow();
        if you != () && marker.gone == () {
            draw_path(you[0], you[2], marker.x, marker.z, now);
        }
        draw_marker(marker, now);
    }

    for rip in ripples { draw_ripple(rip, now); }
    for s in splashes { draw_splash(s, now); }
    if toast != () { draw_toast(toast, now); }

    // Picked here rather than in on_update: on_draw_ui sees the camera this
    // frame renders with, so the reticle can't trail the ground under it.
    let hover = screen_to_ground(mouse_x(), mouse_y(), cap_ground_y());
    if hover != () { draw_reticle(hover, now); }
}

// The Capitol's gold, #c9a227.
fn gold() { [0.788, 0.635, 0.153] }
// Spray off a canal: the water blue of the 2D clients, lifted toward white.
fn spray() { [0.78, 0.88, 1.0] }

// A ring of radius r (m) on the ground at (cx, cz), as projected segments.
fn ring(cx, cz, y, r, c, alpha, width, steps) {
    let prev = ();
    for i in 0..=steps {
        let a = i.to_float() / steps.to_float() * 2.0 * PI();
        let p = world_to_screen(cx + r * a.cos(), y, cz + r * a.sin());
        if !p.visible { prev = (); continue; }
        if prev != () { draw_line(prev.x, prev.y, p.x, p.y, c[0], c[1], c[2], alpha, width); }
        prev = p;
    }
}

// The way you're walking: dashes from your feet to the ring, marching
// toward it, faint, and fading in from under you so they don't cross the
// avatar. Long walks get longer dashes, so the count stays bounded.
fn draw_path(x0, z0, x1, z1, now) {
    let dx = x1 - x0;
    let dz = z1 - z0;
    let len = (dx * dx + dz * dz).sqrt();
    if len < 1.5 { return; }
    let ux = dx / len;
    let uz = dz / len;
    let step = (len / 90.0).max(1.4);
    let dash = step * 0.55;
    let y = cap_ground_y();
    let c = gold();
    let s = (now * 1.6 % 1.0) * step;
    // Stop short of the ring, start clear of your feet.
    let end = len - 1.3;
    while s < end {
        let a = s.max(0.8);
        let b = (s + dash).min(end);
        if b > a {
            let fade = ((a - 0.8) / 2.5).min(1.0);
            let p = world_to_screen(x0 + ux * a, y, z0 + uz * a);
            let q = world_to_screen(x0 + ux * b, y, z0 + uz * b);
            if p.visible && q.visible {
                draw_line(p.x, p.y, q.x, q.y, c[0], c[1], c[2], 0.38 * fade, 1.5);
            }
        }
        s += step;
    }
}

// Where you're going: a gold ring with a soft halo, settling in from wide
// as it appears, and closing to a point as you arrive.
fn draw_marker(m, now) {
    let y = cap_ground_y();
    let c = gold();
    let born = smooth(((now - m.seen) / 0.35).min(1.0));
    let r = 1.0 + 0.8 * (1.0 - born);
    let alpha = 0.9 * born;
    if m.gone != () {
        let k = smooth(((now - m.gone) / marker_fade()).min(1.0));
        r *= 1.0 - 0.8 * k;
        alpha *= 1.0 - k;
    }
    let breathe = 1.0 + 0.05 * (now * 3.0).sin();
    ring(m.x, m.z, y, r * breathe, c, alpha, 2.0, 40);
    ring(m.x, m.z, y, r * 1.8, c, alpha * 0.22, 1.0, 48);
    let p = world_to_screen(m.x, y, m.z);
    if p.visible { draw_circle(p.x, p.y, 2.5, c[0], c[1], c[2], alpha); }
}

// Two gold rings spreading over the ground from where the click landed,
// the second a beat behind, both easing out as they fade.
fn draw_ripple(rip, now) {
    for k in 0..2 {
        let age = (now - rip.born) / ripple_life() - k.to_float() * 0.2;
        if age <= 0.0 || age >= 1.0 { continue; }
        let ease = 1.0 - (1.0 - age) * (1.0 - age);
        let alpha = 0.85 * (1.0 - age) * (1.0 - age);
        ring(rip.x, rip.z, cap_ground_y(), 0.3 + 5.5 * ease, gold(), alpha, 1.5, 48);
    }
}

// Where the canal stopped you: rings spread on the water, quicker and
// tighter than a click's, and a spray of drops goes up and falls back.
fn draw_splash(s, now) {
    let age = now - s.born;
    let k = age / splash_life();
    let c = spray();
    for i in 0..3 {
        let a = k * 1.4 - i.to_float() * 0.18;
        if a <= 0.0 || a >= 1.0 { continue; }
        let ease = 1.0 - (1.0 - a) * (1.0 - a);
        ring(s.x, s.z, 0.05, 0.15 + 1.2 * ease, c, 0.75 * (1.0 - a), 1.5, 36);
    }
    for d in s.drops {
        let t = age * 0.9;
        let y = 0.1 + d.vy * t - 4.9 * t * t;
        if y < 0.0 { continue; }
        let p = world_to_screen(s.x + d.vx * t, y, s.z + d.vz * t);
        if !p.visible { continue; }
        draw_circle(p.x, p.y, d.size * (1.0 - k * 0.5), c[0], c[1], c[2], 0.85 * (1.0 - k));
    }
}

// "Find a bridge", low in the middle of the screen, on the panels' glass,
// with which water stopped you under it. It rises a little as it comes in.
fn draw_toast(toast, now) {
    let age = now - toast.born;
    let a = if age < toast_in() { smooth(age / toast_in()) }
            else if age < toast_in() + toast_hold() { 1.0 }
            else { 1.0 - smooth((age - toast_in() - toast_hold()) / toast_out()) };
    if a <= 0.0 { return; }
    let rise = 10.0 * (1.0 - smooth((age / toast_in()).min(1.0)));
    let title_size = 22.0;
    let body_size = 14.0;
    let spacing = title_size * 0.12;
    let tm = measure_text_ex(toast.title, title_size, "display", spacing);
    let bm = measure_text_ex(toast.body, body_size, "body", 0.0);
    let w = tm.width.max(bm.width) + 48.0;
    let h = tm.height + bm.height + 30.0;
    let x = (screen_width() - w) / 2.0;
    let y = screen_height() * 0.74 + rise;
    // The panels' glass, rgba(15,15,25,0.95), and their edge.
    draw_rect_ex(x, y, w, h, 0.059, 0.059, 0.098, 0.9 * a, 10.0, 0);
    draw_rect_outline_ex(x, y, w, h, 0.314, 0.314, 0.392, 0.3 * a, 1.0, 10.0, 0);
    let c = gold();
    draw_text_opts(x + (w - tm.width) / 2.0, y + 11.0, toast.title, title_size, #{
        font: "display", spacing: spacing, color: [c[0], c[1], c[2], a],
    });
    draw_text_opts(x + (w - bm.width) / 2.0, y + 17.0 + tm.height, toast.body, body_size, #{
        font: "body", color: [0.878, 0.878, 0.878, 0.85 * a],
    });
}

// The reticle: a thin gold ring lying on the ground under the cursor, a
// fainter one outside it, and four ticks pointing north, east, south and
// west, so the ground's own perspective shows in its shape.
fn draw_reticle(g, now) {
    let breathe = 1.0 + 0.06 * (now * 2.4).sin();
    let r = 0.9 * breathe;
    let c = gold();
    let y = cap_ground_y();
    ring(g[0], g[1], y, r, c, 0.9, 1.5, 40);
    ring(g[0], g[1], y, r * 1.9, c, 0.25, 1.0, 56);
    for a in [0.0, 0.5 * PI(), PI(), 1.5 * PI()] {
        let p = world_to_screen(g[0] + r * 1.25 * a.cos(), y, g[1] + r * 1.25 * a.sin());
        let q = world_to_screen(g[0] + r * 1.7 * a.cos(), y, g[1] + r * 1.7 * a.sin());
        if p.visible && q.visible {
            draw_line(p.x, p.y, q.x, q.y, c[0], c[1], c[2], 0.7, 1.5);
        }
    }
}

fn smooth(t) {
    let t = t.max(0.0).min(1.0);
    t * t * (3.0 - 2.0 * t)
}
