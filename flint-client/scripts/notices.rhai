// Notices (P8.2): what the world tells you, drawn on the screen over
// everything else. Two things live here, the toast stack and the pickups.
//
// The toast stack. Every message a click or the server sends back comes
// here as a toast: low in the middle of the screen, on the panels' glass,
// a title in Cinzel gold over a line of body text. A new one comes in at
// the bottom and the older ones slide up out of its way, so two in a row
// both get read. The same message again doesn't stack: the one showing
// takes a count ("×2") and starts its time again. At most four stand at
// once. A fifth sends the oldest out early. A small diamond left of the
// title says what kind of news it is: gold for something gained, ember for
// a refusal, water blue for a canal.
//
// Pickups. When a gather comes in, the yield rises off the tuft
// (input.rhai), and a moment later its grade pips lift off after it and
// fly to the inventory: to the panel if it's open, otherwise to the
// header's Inventory button (cap_ui_anchor). Each pip arcs up and over,
// slow from the tuft and quick at the end, trailing a fading tail. When it
// lands, the panel or button glows gold for a beat (cap_ui_pulse) and a few
// sparks fly off where it hit.
//
// An A-grade item sparkles. Its pips trail four-pointed glints as they
// fly, the yield rising off the tuft twinkles (input.rhai), its slot in
// the inventory glints every few seconds (ui/slots.rs), and a toast names
// it.

// ── State ────────────────────────────────────────────────────────────────

// Each toast: #{ title, body, kind, born, count, leave, lift }. `leave` is
// when it was sent out early, or (). `lift` is how far up the stack it
// stands, in points, eased toward its place.
let toasts = [];
// Each pickup in flight: #{ x, y, z, born, wait, colour, a, bend }.
let flights = [];
// Sparks where a pickup landed, and glints shed by an A grade's.
let sparks = [];
let glints = [];

// ── Tuning ───────────────────────────────────────────────────────────────

// A toast: in, held, out (s). A longer body holds a little longer.
fn toast_in() { 0.25 }
fn toast_hold() { 2.8 }
fn toast_out() { 0.7 }
// The stack: how many, the room between two, and how fast one slides up
// (1/e in 1/k seconds).
fn toast_max() { 4 }
fn toast_gap() { 8.0 }
fn toast_k() { 12.0 }
// A pickup: how long after the yield appears it lifts off, the gap between
// pips, and how long it flies (s). At most this many pips per gather.
fn pickup_wait() { 0.45 }
fn pickup_stagger() { 0.09 }
fn pickup_flight() { 0.9 }
fn pickup_most() { 4 }
// Sparks off a landing, and glints off an A's pip (s).
fn spark_life() { 0.55 }
fn glint_life() { 0.6 }

// ── Per frame ────────────────────────────────────────────────────────────

fn on_update() {
    let t = total_time();
    toasts = toasts.filter(|q| t < toast_end(q));
    sparks = sparks.filter(|s| t - s.born < spark_life());
    glints = glints.filter(|g| t - g.born < glint_life());
    // A pickup that has landed lights where it hit and is done.
    let flying = [];
    for f in flights {
        if t - f.born - f.wait < pickup_flight() { flying.push(f); continue; }
        let end = anchor();
        if end == () { continue; }
        cap_ui_pulse("inventory");
        for i in 0..5 {
            let a = random() * 2.0 * PI();
            let v = random_range(40.0, 110.0);
            sparks.push(#{
                x: end.x, y: end.y, vx: a.cos() * v, vy: a.sin() * v - 30.0,
                born: t, c: f.colour,
            });
        }
    }
    flights = flying;
}

fn on_event(name, data) {
    let t = total_time();
    if name == "extraction_completed" {
        let n = if data.quantity > pickup_most() { pickup_most() } else { data.quantity };
        for i in 0..n {
            flights.push(#{
                x: data.x, y: cap_ground_at(data.x, data.z) + 1.3, z: data.z, born: t,
                wait: pickup_wait() + i.to_float() * pickup_stagger(),
                colour: data.colour, a: data.grade == "A",
                // Each pip bows its own way, so a handful fans out.
                bend: random_range(-0.35, 0.35),
            });
        }
        if data.grade == "A" {
            toasts = add_toast(toasts, "A grade", found(data), "good", t);
        }
        return;
    }
    if name == "crafting_completed" {
        if data.grade == "A" {
            toasts = add_toast(toasts, "A grade", found(data), "good", t);
        }
        return;
    }
    if name == "plot_claimed" {
        toasts = add_toast(toasts, "The deed is yours", "Place stations here", "good", t);
        return;
    }
    if name == "extraction_note" || name == "plot_note" || name == "placement_note"
        || name == "station_note" || name == "trade_note" || name == "tool_note"
        || name == "market_note" {
        toasts = add_toast(toasts, data.title, data.body, "note", t);
        return;
    }
    if name == "movement_blocked" {
        if data.steep == true {
            toasts = add_toast(toasts, "Too steep", "Find a gentler way up", "note", t);
            return;
        }
        let water = data.water;
        let body = water.sub_string(0, 1).to_upper() + water.sub_string(1) + " has no crossing here";
        let title = if water == "the river" { "Find a ford or a bridge" }
                    else if water == "the lake" { "Go round" }
                    else { "Find a bridge" };
        toasts = add_toast(toasts, title, body, "water", t);
    }
}

// "3 Plant Fiber at quality 93".
fn found(data) {
    `${data.quantity} ${data.item_name} at quality ${data.quality}`
}

// ── The toast stack ──────────────────────────────────────────────────────

// The stack with a toast added: the same message again takes a count and
// starts over where it stands, and past the most, the oldest still
// standing is sent out.
fn add_toast(list, title, body, kind, t) {
    for i in 0..list.len() {
        if list[i].title == title && list[i].body == body && list[i].leave == () {
            list[i].count += 1;
            // Back to fully in, without fading in again.
            list[i].born = t - toast_in();
            return list;
        }
    }
    let staying = 0;
    for q in list { if q.leave == () { staying += 1; } }
    if staying >= toast_max() {
        for i in 0..list.len() {
            if list[i].leave == () { list[i].leave = t; break; }
        }
    }
    list.push(#{ title: title, body: body, kind: kind, born: t, count: 1, leave: (), lift: 0.0 });
    list
}

fn toast_hold_for(q) { toast_hold() + (q.body.len().to_float() / 40.0).min(1.5) }

// When it's gone.
fn toast_end(q) {
    let natural = q.born + toast_in() + toast_hold_for(q) + toast_out();
    if q.leave == () { natural } else { natural.min(q.leave + toast_out() * 0.6) }
}

// How much of it shows, 0 to 1.
fn toast_alpha(q, now) {
    let age = now - q.born;
    let a = if age < toast_in() { smooth(age / toast_in()) } else { 1.0 };
    let out_at = toast_end(q) - (if q.leave == () { toast_out() } else { toast_out() * 0.6 });
    if now > out_at { a *= 1.0 - smooth((now - out_at) / (toast_end(q) - out_at)); }
    a
}

// The kind's mark: gold for good news, ember for a refusal, water blue.
fn kind_colour(kind) {
    if kind == "good" { gold() }
    else if kind == "water" { [0.55, 0.75, 1.0] }
    else { [0.91, 0.53, 0.35] }
}

fn title_size() { 22.0 }
// The HUD layer toasts draw on: over everything else the scripts draw.
fn toast_layer() { 10 }
fn body_size() { 14.0 }

// A toast's size on the screen.
fn toast_size(q) {
    let spacing = title_size() * 0.12;
    let tm = measure_text_ex(q.title, title_size(), "display", spacing);
    let bm = measure_text_ex(q.body, body_size(), "body", 0.0);
    let extra = if q.count > 1 { 34.0 } else { 0.0 };
    #{ w: (tm.width + 20.0 + extra).max(bm.width) + 48.0, h: tm.height + bm.height + 30.0,
       tw: tm.width, th: tm.height, bw: bm.width, spacing: spacing, extra: extra }
}

// One toast, its bottom edge at `bottom`.
fn draw_toast(q, bottom, now) {
    let a = toast_alpha(q, now);
    if a <= 0.0 { return; }
    let s = toast_size(q);
    // It rises a little as it comes in.
    let rise = 10.0 * (1.0 - smooth(((now - q.born) / toast_in()).min(1.0)));
    let x = (screen_width() - s.w) / 2.0;
    let y = bottom - s.h + rise;
    // The panels' glass, rgba(15,15,25,0.95), and their edge.
    // Over the marks on the world, whichever script drew them.
    draw_rect_ex(x, y, s.w, s.h, 0.059, 0.059, 0.098, 0.9 * a, 10.0, toast_layer());
    draw_rect_outline_ex(x, y, s.w, s.h, 0.314, 0.314, 0.392, 0.3 * a, 1.0, 10.0, toast_layer());
    let c = gold();
    let k = kind_colour(q.kind);
    let head = x + (s.w - (s.tw + 20.0 + s.extra)) / 2.0;
    // The kind's diamond, then the title.
    let dy = y + 11.0 + s.th / 2.0;
    let d = 4.5;
    draw_polygon([[head + d, dy - d], [head + 2.0 * d, dy], [head + d, dy + d], [head, dy]],
        k[0], k[1], k[2], a, toast_layer() + 1);
    draw_text_opts(head + 20.0, y + 11.0, q.title, title_size(), #{
        font: "display", spacing: s.spacing, color: [c[0], c[1], c[2], a], layer: toast_layer() + 1,
    });
    if q.count > 1 {
        let n = "×" + q.count;
        let px = head + 20.0 + s.tw + 8.0;
        draw_rect_ex(px, dy - 9.0, 28.0, 18.0, c[0], c[1], c[2], 0.18 * a, 9.0, toast_layer() + 1);
        draw_text_opts(px + 14.0, dy - 8.0, n, 13.0, #{
            font: "strong", align: "center", color: [c[0], c[1], c[2], a], layer: toast_layer() + 1,
        });
    }
    draw_text_opts(x + (s.w - s.bw) / 2.0, y + 17.0 + s.th, q.body, body_size(), #{
        font: "body", color: [0.878, 0.878, 0.878, 0.85 * a], layer: toast_layer() + 1,
    });
}

// ── Pickups ──────────────────────────────────────────────────────────────

// Where a pickup lands: the top of the inventory panel if it's open, else
// the middle of the header's Inventory button. () when neither is up. The
// panel and the header are egui's, so pickups are drawn over the UI
// (push_over_ui, Flint ADR 0085) and land on them.
fn anchor() {
    let r = cap_ui_anchor("inventory");
    if r == () { return (); }
    if r.panel { #{ x: r.x + r.w / 2.0, y: r.y + 22.0 } }
    else { #{ x: r.x + r.w / 2.0, y: r.y + r.h / 2.0 } }
}

// Where a pickup is at k (0 to 1 along its flight): on a curve from the
// tuft up over and down into the anchor, bowed to one side, and never
// over the top of the screen.
fn flight_at(sx, sy, ex, ey, bend, k) {
    let dx = ex - sx;
    let dy = ey - sy;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    // Up from the tuft, and bowed off the straight line.
    let cx = sx + dx * 0.35 - dy / len * len * bend;
    let cy = (sy.min(ey) - 0.25 * len - 40.0).max(0.0);
    let u = 1.0 - k;
    [u * u * sx + 2.0 * u * k * cx + k * k * ex, u * u * sy + 2.0 * u * k * cy + k * k * ey]
}

// Slow off the tuft, quickening into the landing.
fn flight_ease(k) { k * k * (2.2 - 1.2 * k) }

fn draw_flight(f, end, now) {
    let k = (now - f.born - f.wait) / pickup_flight();
    let c = f.colour;
    let start = world_to_screen(f.x, f.y, f.z);
    if !start.visible { return; }
    if k < 0.0 {
        // Waiting to lift off: a pip swelling over the tuft.
        let w = smooth((k + 0.25) / 0.25);
        if w > 0.0 { draw_circle(start.x, start.y - 14.0, 6.0 * w, c[0], c[1], c[2], 0.9 * w); }
        return;
    }
    let sy = start.y - 14.0;
    // The tail: where it was a moment ago, smaller and fainter.
    for j in 1..7 {
        let kj = (k - j.to_float() * 0.035).max(0.0);
        let p = flight_at(start.x, sy, end.x, end.y, f.bend, flight_ease(kj));
        let fade = 1.0 - j.to_float() / 7.0;
        draw_circle(p[0], p[1], (6.0 - 3.0 * k) * fade, c[0], c[1], c[2], 0.45 * fade);
    }
    let p = flight_at(start.x, sy, end.x, end.y, f.bend, flight_ease(k));
    let r = 6.0 - 2.5 * k;
    draw_circle(p[0], p[1], r + 2.0, 0.0, 0.0, 0.0, 0.3);
    draw_circle(p[0], p[1], r, c[0], c[1], c[2], 1.0);
    draw_circle(p[0] - r * 0.3, p[1] - r * 0.3, r * 0.35, 1.0, 1.0, 0.95, 0.6);
    if f.a { draw_glint(p[0], p[1], 9.0 + 3.0 * (now * 14.0).sin(), now * 2.0, 0.9); }
}

// A four-pointed glint, `r` points from its middle to a tip, turned by
// `turn`: two crossed slim diamonds, gold-white.
fn draw_glint(x, y, r, turn, a) {
    if r <= 0.5 || a <= 0.0 { return; }
    let w = r * 0.22;
    for k in 0..2 {
        let t = turn + k.to_float() * 0.5 * PI();
        let ux = t.cos();
        let uy = t.sin();
        draw_polygon([
            [x + ux * r, y + uy * r], [x - uy * w, y + ux * w],
            [x - ux * r, y - uy * r], [x + uy * w, y - ux * w],
        ], 1.0, 0.95, 0.7, a, 2);
    }
    draw_circle(x, y, w * 1.2, 1.0, 1.0, 1.0, a);
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn on_draw_ui() {
    if cap_at_login() { return; }
    let now = total_time();
    let dt = delta_time().min(0.1);

    // Pickups fly over the panels and the header, to land on them.
    push_over_ui();
    let end = anchor();
    if end != () {
        for f in flights {
            draw_flight(f, end, now);
            // An A's pip sheds glints along its way.
            let k = (now - f.born - f.wait) / pickup_flight();
            if f.a && k > 0.0 && k < 1.0 && random() < 0.35 {
                let s = world_to_screen(f.x, f.y, f.z);
                if s.visible {
                    let p = flight_at(s.x, s.y - 14.0, end.x, end.y, f.bend, flight_ease(k));
                    glints.push(#{ x: p[0] + random_range(-8.0, 8.0), y: p[1] + random_range(-8.0, 8.0),
                                   born: now, turn: random() * PI() });
                }
            }
        }
    }
    for g in glints {
        let k = (now - g.born) / glint_life();
        draw_glint(g.x, g.y, 7.0 * (1.0 - (2.0 * k - 1.0).abs()), g.turn + k, 0.8 * (1.0 - k));
    }
    for s in sparks {
        let age = now - s.born;
        let k = age / spark_life();
        let x = s.x + s.vx * age;
        let y = s.y + s.vy * age + 160.0 * age * age;
        draw_circle(x, y, 2.5 * (1.0 - k), s.c[0], s.c[1], s.c[2], 1.0 - k);
    }
    pop_over_ui();

    // The stack, newest at the bottom. Each eases toward its place above
    // the ones newer than it, so a new toast pushes the rest up.
    // The trade window (P7.3) stands where toasts go, so while it's open
    // they stack above the avatars instead.
    let trade = cap_trade();
    let low = if trade != () && trade.state == "open" { 0.30 } else { 0.74 };
    let bottom = screen_height() * low + 60.0;
    let above = 0.0;
    let i = toasts.len() - 1;
    while i >= 0 {
        let q = toasts[i];
        let s = toast_size(q);
        // A toast on its way out doesn't hold its room.
        let room = (s.h + toast_gap()) * toast_alpha(q, now).max(if q.leave == () { 1.0 } else { 0.0 });
        let to = above;
        q.lift += (to - q.lift) * (1.0 - (-toast_k() * dt).exp());
        toasts[i] = q;
        draw_toast(q, bottom - q.lift, now);
        above += room;
        i -= 1;
    }
}

// The Capitol's gold, #c9a227.
fn gold() { [0.788, 0.635, 0.153] }

fn smooth(t) {
    let t = t.max(0.0).min(1.0);
    t * t * (3.0 - 2.0 * t)
}
