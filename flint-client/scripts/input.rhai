// Input to intents (plan §4.2): what a left click on the world means, and
// the marks it leaves. P4.3 is click to move, P4.4 gathering, P6.3 stations.
//
// A click is picked off the ground with screen_to_ground and offered to
// each intent in the React client's order (GameCanvas.tsx handleClick).
// The first that takes it wins:
//
//   1. placing a station: put it here              (P6.2)
//   2. shift-click a player: trade, or walk closer  (P7.3)
//   3. a station: open it, or walk closer           (P6.3)
//   4. alt-click a plot: select it                  (P6.4, here)
//   5. an available resource node: extract          (P4.4, here)
//   6. anywhere else: walk there                    (P4.3, here)
//
// Each is a function below that gets the click (#{ x, z } in world
// metres, `shift`, `alt`) and returns true when it took it.
//
// Walking: cap_move(x, z) sends you (clamped to the world, 0-8000 server
// units), and cap_destination() is where you're going until you arrive.
// A gold ring lies there, with a faint dashed line marching to it from
// your feet. The click itself drops a ripple, as the spike's did (P0.4).
//
// The server stops a walk at a canal's edge with `movement_blocked`. Rust
// fires it here with where you stand and which way you were going. The
// water splashes just ahead of you, the camera rig gives a small nudge
// (camera.rhai), and a toast says to find a bridge (notices.rhai, which
// holds every toast).
//
// Gathering: a click on a grass tuft (cap_node_at) asks Rust to extract it
// (cap_extract). Within 50 units (5 m) that's at once; further, you walk
// there and it starts as you arrive (capitol_core::harvest). While you
// walk, a gold sight turns round the tuft and the dashes run to it. While
// anyone extracts, a ring fills round the tuft: gold for you, pale green
// for others. On completion "+N fiber" rises from it with the grade on a
// pip of its colour, twinkling if it's an A, and the pips fly off to
// the inventory (notices.rhai). A tuft cut down throws up bits of blade; one that
// grows back breathes out a green ring. Hovering a tuft you can take
// draws the reticle round it.
//
// Crafting (P5.5) has no click of its own: the panel sends it. While
// anyone crafts, a purple ring fills round their feet (cap_players'
// `craft`), with two faint strands winding round it opposite ways, as
// fibre twists into rope. Yours is bright, others' fainter. When yours
// finishes, a purple ring breathes out from you and what you made rises
// over your name, as a gather's yield rises off its tuft. Walking cancels
// a craft on the server, so a click on the ground while crafting stops it.
//
// Stations (P6.1). Within reach of one, 75 units (7.5 m) by
// capitol_core::station, an orange ring (#f97316, React's in-range ring)
// closes in round it, with four short marks turning slowly outside it,
// and opens out again as you leave. Hovering one out of reach draws the
// ring faintly, so you can see it will take a click. Like every ring here
// it's screen-space lines with no depth test, so its far side crosses the
// model's front (issue #86).
//
// A click inside a station's ring (cap_station_at) opens it (P6.3,
// cap_open_station). Within reach that's at once. Further away, you walk
// to its near side and it opens as you arrive (capitol_core::station). The
// panel is Rust's (ui/station.rs). Walking out of reach closes it, and a
// refusal ("Not your station") comes back as a `station_note` toast.
//
// Placing a station (P6.2): a right click on a kit in the inventory starts
// it. Rust draws the ghost, the station itself, tinted and translucent,
// and says through cap_placing() whether it can go where the cursor is.
// Here go the marks round it: a ring under the ghost, green or red (React's
// #22c55e and #ef4444), the reason under it when it's red, your plots
// outlined in gold, a dashed ring of how far from you it may go, and a ring
// round each station it's too close to. Every left click is the
// placement's while it lasts: one where it can go is sent (cap_place),
// one where it can't is refused with a toast. A right click or Esc
// (Rust, extension.rs) stops it, and clicks walk again.
//
// Trading (P7.3): shift-click someone (cap_player_at, cap_trade_with) to
// ask them to trade. Within 75 units (7.5 m) the request goes at once;
// further, you walk over and it goes as you arrive. Holding shift over
// someone rings their feet in the Trade District's blue, so you can see the
// click will ask. While a request is out, or in, a faint strand runs
// between you, its light flowing from whoever asked. Once the window is
// open it's a twisted strand at hand height, two fibres winding round
// each other, and each end turns gold as that side accepts: a deal is
// rope made between two people. When it goes through, a gold pulse runs
// both ways along it and rings breathe out round you both. The window
// itself is Rust's (ui/trade.rs), and refusals and endings come here as
// `trade_note` toasts.
//
// Plots (P6.4): alt-click a lot to see it in the plot panel (Rust asks
// the server what's there, cap_pick_plot). While the panel is open, the
// picked lot's edge is traced on the ground: a fine line with a brighter
// stretch running round it, gold for yours, pale stone for anyone else's
// or for sale. Claiming one lets the world answer: a gold ring breathes
// out over the lot from its middle, a second just behind, and the edge
// flares as the gold settles into its curb (world::plots::owned).

// The right button is the camera's (a right-drag orbits), so this is the
// left button only. egui keeps any click over its panels.

let ripples = [];
// The ring at the destination: #{ x, z, seen } while it's there, then
// `gone` from when you arrived, as it fades.
let marker = ();
let splashes = [];
// "+N fiber" rising from a tuft, bits of a cut one, and a regrown one's ring.
let yields = [];
let cuttings = [];
let sprouts = [];
// A finished craft's ring, breathing out from where you stood.
let blooms = [];
// A plot just claimed: its rings breathing out over it.
let deeds = [];
// A trade just made: #{ x, z, tx, tz, born } for the pulse and the rings.
let trades = [];

// ── Tuning ───────────────────────────────────────────────────────────────

// How long a click's ripple takes to spread and fade, in seconds.
fn ripple_life() { 1.6 }
// How long the destination ring takes to close once you arrive.
fn marker_fade() { 0.45 }
// The splash: how long, and how far past where you stopped (m).
fn splash_life() { 1.1 }
fn splash_ahead() { 1.1 }
// The yield rising off a tuft (s), and how far (m).
fn yield_life() { 2.4 }
fn yield_rise() { 1.1 }
// Bits of a cut tuft, and a regrown one's ring (s).
fn cutting_life() { 0.9 }
fn sprout_life() { 1.4 }
// A finished craft's ring (s).
fn bloom_life() { 1.3 }
// A claimed plot's rings, and the flare on the picked lot's edge (s).
fn deed_life() { 2.2 }
// A trade's pulse and rings (s), and the strand's height over the feet (m).
fn trade_life() { 1.8 }
fn strand_height() { 1.0 }
// The ring round someone crafting (m). A craft's yield starts where your
// name hangs (entities.rs LABEL_Y, m over your feet) and that many points
// higher on the screen, so it clears the name at any zoom.
fn craft_ring() { 0.95 }
fn name_height() { 2.25 }
fn craft_lift() { 30.0 }
// The ring round a tuft being worked, and the sight round your target (m).
fn work_ring() { 1.15 }
// A ring sized to the node: a tuft's own, or clear of a trunk, a boulder or
// a bank (its `r`, E2.5), never tighter than `base`.
fn node_ring(n, base) {
    if n.r == () { return base; }
    if n.r > base { n.r } else { base }
}
// How strongly a hovered station's ring shows when it's out of reach.
fn hover_reach() { 0.4 }
fn sight_ring() { 1.0 }

// ── Per frame ────────────────────────────────────────────────────────────

fn on_update() {
    let t = total_time();
    ripples = ripples.filter(|r| t - r.born < ripple_life());
    splashes = splashes.filter(|s| t - s.born < splash_life());
    yields = yields.filter(|y| t - y.born < yield_life());
    cuttings = cuttings.filter(|c| t - c.born < cutting_life());
    sprouts = sprouts.filter(|s| t - s.born < sprout_life());
    blooms = blooms.filter(|b| t - b.born < bloom_life());
    deeds = deeds.filter(|d| t - d.born < deed_life());
    trades = trades.filter(|d| t - d.born < trade_life());
    if cap_at_login() { return; }
    // The ghost follows the ground under the cursor.
    if cap_placing() != () {
        let h = screen_to_ground(mouse_x(), mouse_y(), cap_ground_y());
        if h == () { cap_place_off(); } else { cap_place_hover(h[0], h[1]); }
    }
    if !is_mouse_just_pressed(0) { return; }

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

// ── The intents above walking ─────────────────────────────────────────────

// Shift-click someone: ask them to trade, or walk over and ask. Shift
// over no one falls through, as React's does.
fn trade_with(click) {
    if !click.shift { return false; }
    let p = cap_player_at(click.x, click.z);
    if p == () { return false; }
    cap_trade_with(p.id);
    true
}
fn open_station(click) {           // P6.3
    let s = cap_station_at(click.x, click.z);
    if s == () { return false; }
    cap_open_station(s.id);
    true
}

// Alt-click: the plot here, in the panel. Over a road or open ground the
// server finds none, and a toast says so.
fn select_plot(click) {
    if !click.alt { return false; }
    cap_pick_plot(click.x, click.z);
    true
}

// Placing a station: every click is the placement's. Rust sends it if it
// can go there, and refuses it with a placement_note if it can't.
fn place_station(click) {
    if cap_placing() == () { return false; }
    cap_place(click.x, click.z);
    true
}

// A tuft you can take: extract it, or walk there and extract. Anything
// else (a cut tuft, one someone is working) falls through to walking.
fn extract(click) {
    let n = cap_node_at(click.x, click.z);
    if n == () || n.state != "available" { return false; }
    cap_extract(n.id);
    true
}

// ── Server happenings ────────────────────────────────────────────────────

fn on_event(name, data) {
    let t = total_time();
    if name == "extraction_completed" {
        yields.push(#{
            x: data.x, z: data.z, born: t, base: cap_ground_y() + 1.3, up: 0.0,
            text: "+" + data.quantity + " " + data.item_name,
            grade: data.grade, colour: data.colour,
        });
        return;
    }
    if name == "crafting_completed" {
        let you = cap_follow();
        let feet = if you != () { you[1] } else { cap_ground_y() };
        yields.push(#{
            x: data.x, z: data.z, born: t, base: feet + name_height(), up: craft_lift(),
            text: "+" + data.quantity + " " + data.item_name,
            grade: data.grade, colour: data.colour,
        });
        blooms.push(#{ x: data.x, z: data.z, y: feet + 0.03, born: t });
        return;
    }
    if name == "plot_claimed" {
        deeds.push(#{ x: data.x, z: data.z, r: data.r, born: t });
        return;
    }
    if name == "trade_executed" {
        trades.push(#{ x: data.x, z: data.z, tx: data.tx, tz: data.tz, born: t });
        return;
    }
    if name == "node_depleted" {
        let bits = [];
        for i in 0..18 {
            let a = random() * 2.0 * PI();
            let out = random_range(0.3, 1.2);
            bits.push(#{
                vx: a.cos() * out, vz: a.sin() * out,
                vy: random_range(1.6, 3.2),
                spin: random_range(-9.0, 9.0), turn: random() * PI(),
                len: random_range(5.0, 10.0),
            });
        }
        let colour = if data.r == () { [0.42, 0.64, 0.47] } else { [data.r, data.g, data.b] };
        cuttings.push(#{ x: data.x, z: data.z, born: t, bits: bits, colour: colour });
        return;
    }
    if name == "node_regenerated" {
        sprouts.push(#{ x: data.x, z: data.z, born: t });
        return;
    }
    if name != "movement_blocked" { return; }
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
    // Picked here rather than in on_update: on_draw_ui sees the camera this
    // frame renders with, so the reticle can't trail the ground under it.
    let hover = screen_to_ground(mouse_x(), mouse_y(), cap_ground_y());
    let placing = cap_placing();
    if placing != () { draw_placement(placing, now); }
    // While placing, a station under the cursor is in the ghost's way, not
    // a thing to open.
    let over = if hover != () && placing == () { cap_station_at(hover[0], hover[1]) } else { () };
    for s in cap_stations() {
        // One under the cursor shows its ring, faintly if it's out of reach.
        if over != () && over.id == s.id && s.reach < hover_reach() { s.reach = hover_reach(); }
        if s.reach > 0.01 { draw_station_ring(s, now); }
    }
    // The tuft you're going to gather, if any. While you walk to it, the
    // sight round it stands in for the ring at your destination.
    let target = ();
    for n in cap_nodes() {
        if n.targeted { target = n; }
        if n.progress != () { draw_work(n, now); }
    }
    for p in cap_players() {
        if p.craft != () { draw_craft(p, now); }
    }
    for b in blooms { draw_bloom(b, now); }
    let picked = cap_picked_plot();
    if picked != () {
        let flare = 0.0;
        for d in deeds { flare = flare.max(1.0 - (now - d.born) / deed_life()); }
        draw_plot_edge(picked, now, flare);
    }
    for d in deeds { draw_deed(d, now); }
    let you = cap_follow();
    let trade = cap_trade();
    if trade != () && trade.x != () && you != () && trade.state != "walking" {
        draw_strand(you, trade, now);
    }
    for d in trades { draw_traded(d, you, now); }
    if hover != () && placing == () && is_action_pressed("with_shift") {
        let p = cap_player_at(hover[0], hover[1]);
        if p != () { draw_trade_ring(p, now); }
    }
    if target != () {
        if you != () { draw_path(you[0], you[2], target.x, target.z, now, 0.6); }
        draw_sight(target, now);
    } else if marker != () {
        if you != () && marker.gone == () {
            draw_path(you[0], you[2], marker.x, marker.z, now, 0.38);
        }
        draw_marker(marker, now);
    }
    for c in cuttings { draw_cutting(c, now); }
    for s in sprouts { draw_sprout(s, now); }
    for y in yields { draw_yield(y, now); }

    for rip in ripples { draw_ripple(rip, now); }
    for s in splashes { draw_splash(s, now); }

    if hover != () && placing == () && over == () {
        let n = cap_node_at(hover[0], hover[1]);
        if n != () && n.state == "available" && !n.targeted { draw_reticle_on(n, now); }
        else { draw_reticle(hover, now); }
    }
}

// The Capitol's gold, #c9a227.
fn gold() { [0.788, 0.635, 0.153] }
// Spray off a canal: the water blue of the 2D clients, lifted toward white.
fn spray() { [0.78, 0.88, 1.0] }

// React's in-range orange, #f97316.
fn station_orange() { [0.976, 0.451, 0.086] }
// React's placement preview: #22c55e where it can go, #ef4444 where not.
fn place_green() { [0.133, 0.773, 0.369] }
fn place_red() { [0.937, 0.267, 0.267] }

// Everything round the ghost while a station is placed.
fn draw_placement(p, now) {
    let y = cap_mark_y();
    // Your plots: gold outlines, breathing, so the ground you may build on
    // stands out from the lots around it.
    let lot_y = cap_mark_y();
    let glow = 0.5 + 0.12 * (now * 2.0).sin();
    for q in p.plots {
        for i in 0..4 {
            let a = q[i];
            let b = q[(i + 1) % 4];
            ground_line(a[0], a[1], b[0], b[1], lot_y, gold(), glow, 2.0);
        }
    }
    let c = if p.ok { place_green() } else { place_red() };
    let ghost = p.x != ();
    // How far from you it may go: a slow-turning dashed ring, red while
    // the ghost is past it.
    if p.you_x != () {
        let far = ghost && dist(p.x, p.z, p.you_x, p.you_z) > p.reach;
        let rc = if far { place_red() } else { gold() };
        let ra = if far { 0.6 } else { 0.3 };
        dashed_ring(p.you_x, p.you_z, y, p.reach, rc, ra, 1.5, 48, now * 0.08);
    }
    if !ghost { return; }
    // Each station it's too close to: the ground it keeps clear, red when
    // the ghost is inside it, faint otherwise.
    for s in p.crowding {
        let near = dist(p.x, p.z, s[0], s[1]) < p.spacing;
        let sc = if near { place_red() } else { [0.9, 0.9, 0.9] };
        let sa = if near { 0.55 } else { 0.15 };
        ring(s[0], s[1], y, p.spacing, sc, sa, 1.5, 56);
    }
    // React's preview ring, round the ghost. Asking, it pulses in.
    let r = p.radius + 0.3;
    if p.asking {
        let k = (now * 2.5) % 1.0;
        ring(p.x, p.z, y, r * (1.0 - 0.35 * k), c, 0.8 * (1.0 - k), 2.0, 48);
        ring(p.x, p.z, y, r, c, 0.5, 2.0, 48);
    } else {
        ring(p.x, p.z, y, r, c, 0.75, 2.5, 48);
        ring(p.x, p.z, y, r + 0.35, c, 0.2, 1.0, 52);
    }
    // Under it: the kit, and what a click will do or why it can't.
    let at = world_to_screen(p.x, y, p.z);
    if !at.visible { return; }
    let line = if p.asking { "Placing…" }
               else if p.why != () { p.why }
               else { "Click to place" };
    let lc = if p.why != () && !p.asking { [1.0, 0.62, 0.62] } else { [0.95, 0.92, 0.82] };
    let size = 15.0;
    let m = measure_text_ex(line, size, "strong", 0.0);
    let top = at.y + 26.0;
    draw_text_opts(at.x - m.width / 2.0 + 1.0, top + 1.0, line, size, #{
        font: "strong", color: [0.0, 0.0, 0.0, 0.6],
    });
    draw_text_opts(at.x - m.width / 2.0, top, line, size, #{
        font: "strong", color: [lc[0], lc[1], lc[2], 0.95],
    });
    let hint = p.kit + " · right-click or Esc to stop";
    let hm = measure_text_ex(hint, 12.0, "body", 0.0);
    draw_text_opts(at.x - hm.width / 2.0, top + m.height + 4.0, hint, 12.0, #{
        font: "body", color: [0.88, 0.88, 0.88, 0.6],
    });
}

fn dist(ax, az, bx, bz) {
    let dx = ax - bx;
    let dz = az - bz;
    (dx * dx + dz * dz).sqrt()
}

// Marks on the ground are world lines (#86): drawn in the 3D frame, so
// whatever stands in front of them hides them. Lay them at cap_mark_y().

// A straight line along the ground from (ax, az) to (bx, bz).
fn ground_line(ax, az, bx, bz, y, c, alpha, width) {
    draw_world_line(ax, y, az, bx, y, bz, c[0], c[1], c[2], alpha, width);
}

// A ring of `dashes` dashes, turned by `turn` radians.
fn dashed_ring(cx, cz, y, r, c, alpha, width, dashes, turn) {
    let step = 2.0 * PI() / dashes.to_float();
    for i in 0..dashes {
        let a = turn + i.to_float() * step;
        arc(cx, cz, y, r, a, a + step * 0.55, c, alpha, width);
    }
}

// The ring round a station you can reach: it closes in from a little wider
// and brightens as `reach` eases up, React's half-alpha orange at full,
// with four marks outside it turning a quarter turn every eight seconds.
fn draw_station_ring(s, now) {
    let y = cap_mark_y();
    let c = station_orange();
    let k = s.reach;
    let r = s.radius * (1.2 - 0.2 * k);
    ring(s.x, s.z, y, r, c, 0.5 * k, 2.0, 56);
    let spin = now * 0.196;
    for i in 0..4 {
        let a = spin + i.to_float() * 0.5 * PI();
        arc(s.x, s.z, y, r + 0.22, a - 0.16, a + 0.16, c, 0.4 * k, 1.5);
    }
}

// A ring of radius r (m) on the ground at (cx, cz), in `steps` segments.
fn ring(cx, cz, y, r, c, alpha, width, steps) {
    arc_steps(cx, cz, y, r, 0.0, 2.0 * PI(), c, alpha, width, steps);
}

// An arc from a0 to a1 in `steps` world-line segments.
fn arc_steps(cx, cz, y, r, a0, a1, c, alpha, width, steps) {
    let px = cx + r * a0.cos();
    let pz = cz + r * a0.sin();
    for i in 1..=steps {
        let a = a0 + (a1 - a0) * i.to_float() / steps.to_float();
        let qx = cx + r * a.cos();
        let qz = cz + r * a.sin();
        draw_world_line(px, y, pz, qx, y, qz, c[0], c[1], c[2], alpha, width);
        px = qx;
        pz = qz;
    }
}

// The way you're walking: dashes from your feet to the ring, marching
// toward it, faint, and fading in from under you so they don't cross the
// avatar. Long walks get longer dashes, so the count stays bounded.
fn draw_path(x0, z0, x1, z1, now, strength) {
    let dx = x1 - x0;
    let dz = z1 - z0;
    let len = (dx * dx + dz * dz).sqrt();
    if len < 1.5 { return; }
    let ux = dx / len;
    let uz = dz / len;
    let step = (len / 90.0).max(1.4);
    let dash = step * 0.55;
    let y = cap_mark_y();
    let c = gold();
    let s = (now * 1.6 % 1.0) * step;
    // Stop short of the ring, start clear of your feet.
    let end = len - 1.3;
    while s < end {
        let a = s.max(0.8);
        let b = (s + dash).min(end);
        if b > a {
            let fade = ((a - 0.8) / 2.5).min(1.0);
            draw_world_line(x0 + ux * a, y, z0 + uz * a, x0 + ux * b, y, z0 + uz * b,
                            c[0], c[1], c[2], strength * fade, 1.5);
        }
        s += step;
    }
}

// Where you're going: a gold ring with a soft halo, settling in from wide
// as it appears, and closing to a point as you arrive.
fn draw_marker(m, now) {
    let y = cap_mark_y();
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

// An arc of a ring on the ground, from angle a0 to a1 (radians, 0 east,
// a quarter turn south), 48 segments to the full turn.
fn arc(cx, cz, y, r, a0, a1, c, alpha, width) {
    let steps = ((a1 - a0).abs() / (2.0 * PI()) * 48.0).to_int() + 2;
    arc_steps(cx, cz, y, r, a0, a1, c, alpha, width, steps);
}

// The tuft you're walking to: four gold arcs turning slowly round it, a
// faint ring inside them, breathing. React's targeting ring, with aim.
fn draw_sight(n, now) {
    let y = cap_mark_y();
    let c = gold();
    let r = node_ring(n, sight_ring()) * (1.0 + 0.06 * (now * 3.0).sin());
    let spin = now * 0.9;
    for k in 0..4 {
        let a = spin + k.to_float() * 0.5 * PI();
        arc(n.x, n.z, y, r, a - 0.5, a + 0.5, c, 0.95, 2.0);
    }
    ring(n.x, n.z, y, r * 0.82, c, 0.3, 1.0, 40);
}

// The reticle over a tuft you could take: it settles round the tuft
// rather than lying under the cursor, so you can see the click will gather.
fn draw_reticle_on(n, now) {
    let y = cap_mark_y();
    let c = gold();
    let r = node_ring(n, sight_ring()) * (1.0 + 0.04 * (now * 2.4).sin());
    ring(n.x, n.z, y, r, c, 0.7, 1.5, 44);
    ring(n.x, n.z, y, r * 1.3, c, 0.2, 1.0, 52);
}

// An extraction under way: a track round the tuft, filling clockwise from
// north as it goes. Gold when it's yours, a pale green when it's someone
// else's, with a bead at the leading edge.
fn draw_work(n, now) {
    let y = cap_mark_y();
    let r = node_ring(n, work_ring());
    let c = if n.mine { gold() } else { [0.66, 0.84, 0.69] };
    let w = if n.mine { 3.0 } else { 2.0 };
    let a = if n.mine { 1.0 } else { 0.7 };
    ring(n.x, n.z, y, r, c, 0.22 * a, w, 48);
    let start = -0.5 * PI();
    let end = start + n.progress * 2.0 * PI();
    arc(n.x, n.z, y, r, start, end, c, 0.95 * a, w);
    let p = world_to_screen(n.x + r * end.cos(), y, n.z + r * end.sin());
    if p.visible { draw_circle(p.x, p.y, w * 1.1, c[0], c[1], c[2], a); }
}

// What a gather brought in, rising off the tuft and fading: "+3 Plant
// Fiber" with its grade on a pip in the grade's colour.
fn draw_yield(y, now) {
    let age = now - y.born;
    let k = age / yield_life();
    let rise = yield_rise() * (1.0 - (1.0 - k) * (1.0 - k));
    let p = world_to_screen(y.x, y.base + rise, y.z);
    if !p.visible { return; }
    p.y -= y.up;
    let a = if age < 0.2 { smooth(age / 0.2) } else { 1.0 - smooth((k - 0.6) / 0.4) };
    if a <= 0.0 { return; }
    let size = 17.0;
    let m = measure_text_ex(y.text, size, "strong", 0.0);
    let pip = 18.0;
    let gap = 6.0;
    let w = m.width + gap + pip;
    let x = p.x - w / 2.0;
    let top = p.y - m.height / 2.0;
    // A soft shadow under the words, so they read over grass and road.
    draw_text_opts(x + 1.0, top + 1.0, y.text, size, #{
        font: "strong", color: [0.0, 0.0, 0.0, 0.6 * a],
    });
    draw_text_opts(x, top, y.text, size, #{
        font: "strong", color: [0.95, 0.92, 0.82, a],
    });
    let c = y.colour;
    let px = x + m.width + gap;
    let py = p.y - pip / 2.0;
    draw_rect_ex(px, py, pip, pip, c[0], c[1], c[2], 0.95 * a, pip / 2.0, 0);
    draw_rect_outline_ex(px, py, pip, pip, 0.0, 0.0, 0.0, 0.3 * a, 1.0, pip / 2.0, 0);
    let g = measure_text_ex(y.grade, 12.0, "strong", 0.0);
    draw_text_opts(px + (pip - g.width) / 2.0, p.y - g.height / 2.0, y.grade, 12.0, #{
        font: "strong", color: [0.06, 0.06, 0.1, a],
    });
    if y.grade == "A" { draw_twinkle(px + pip / 2.0, py + pip / 2.0, pip, now - y.born, a); }
}

// An A's pip twinkling: three glints taking turns round it, each swelling
// and gone in under half a second, the first as the yield appears.
fn draw_twinkle(cx, cy, size, age, a) {
    for i in 0..3 {
        let k = (age * 1.6 + i.to_float() / 3.0) % 1.0;
        let swell = (1.0 - (2.0 * k / 0.7 - 1.0).abs()).max(0.0);
        if swell <= 0.0 { continue; }
        let around = i.to_float() * 2.1 + age * 0.6;
        let x = cx + around.cos() * size * 0.85;
        let y = cy + around.sin() * size * 0.85;
        let r = size * 0.45 * swell;
        let w = r * 0.22;
        draw_polygon([[x, y - r], [x + w, y], [x, y + r], [x - w, y]], 1.0, 0.95, 0.7, a * swell, 2);
        draw_polygon([[x - r, y], [x, y - w], [x + r, y], [x, y + w]], 1.0, 0.95, 0.7, a * swell, 2);
    }
}

// A node taken: bits flung up and out, turning as they fall. Blades of a
// tuft, chips of wood or stone, clods of clay, in its kind's colour.
fn draw_cutting(c, now) {
    let age = now - c.born;
    let k = age / cutting_life();
    let green = c.colour;
    for b in c.bits {
        let t = age * 0.9;
        let y = cap_ground_y() + 0.3 + b.vy * t - 4.9 * t * t;
        if y < cap_ground_y() { continue; }
        let p = world_to_screen(c.x + b.vx * t, y, c.z + b.vz * t);
        if !p.visible { continue; }
        let a = b.turn + b.spin * t;
        let half = b.len * 0.5 * (1.0 - k * 0.4);
        draw_line(p.x - a.cos() * half, p.y - a.sin() * half,
                  p.x + a.cos() * half, p.y + a.sin() * half,
                  green[0], green[1], green[2], 0.9 * (1.0 - k), 2.0);
    }
}

// The crafting purple, #8b5cf6, and the lilac of its strands.
fn craft_purple() { [0.545, 0.361, 0.965] }
fn lilac() { [0.80, 0.72, 1.0] }

// A craft under way: a purple track round the crafter's feet, filling
// clockwise from north, a bead at its leading edge, and two strands
// winding round outside it opposite ways.
fn draw_craft(p, now) {
    let y = feet_mark(p.feet_y);
    let r = craft_ring();
    let c = craft_purple();
    let a = p.alpha * (if p.me { 1.0 } else { 0.6 });
    let w = if p.me { 3.0 } else { 2.0 };
    ring(p.x, p.z, y, r, c, 0.22 * a, w, 48);
    let start = -0.5 * PI();
    let end = start + p.craft * 2.0 * PI();
    if p.craft > 0.0 { arc(p.x, p.z, y, r, start, end, c, 0.95 * a, w); }
    let b = world_to_screen(p.x + r * end.cos(), y, p.z + r * end.sin());
    if b.visible { draw_circle(b.x, b.y, w * 1.1, c[0], c[1], c[2], a); }
    let l = lilac();
    for k in 0..2 {
        let turn = if k == 0 { 1.0 } else { -1.0 };
        let s = now * 1.7 * turn + k.to_float() * PI();
        arc(p.x, p.z, y, r * 1.2, s, s + 1.1, l, 0.4 * a, 1.0);
    }
}

// A craft finished: a purple ring breathing out from where you stand,
// with a fainter one just behind it.
fn draw_bloom(b, now) {
    let k = (now - b.born) / bloom_life();
    let ease = 1.0 - (1.0 - k) * (1.0 - k);
    let c = craft_purple();
    ring(b.x, b.z, b.y, craft_ring() + 1.4 * ease, c, 0.8 * (1.0 - k), 2.0, 44);
    let k2 = (k - 0.15).max(0.0) / 0.85;
    let e2 = 1.0 - (1.0 - k2) * (1.0 - k2);
    ring(b.x, b.z, b.y, craft_ring() + 0.9 * e2, lilac(), 0.4 * (1.0 - k2), 1.0, 40);
}

// The Trade District's blue, #3b82f6, and the trade window's edge.
fn trade_blue() { [0.231, 0.510, 0.965] }

// Someone shift would ask: a blue ring at their feet, closing in a little
// as it breathes, with a fainter one outside.
fn draw_trade_ring(p, now) {
    let y = feet_mark(p.feet_y);
    let c = trade_blue();
    let r = 0.85 + 0.05 * (now * 3.0).sin();
    ring(p.x, p.z, y, r, c, 0.85, 2.0, 40);
    ring(p.x, p.z, y, r * 1.45, c, 0.25, 1.0, 48);
}

// Under a player's feet: a little over them, and never under the marks'
// layer, so a lot or a road doesn't hide the ring.
fn feet_mark(feet_y) { (feet_y + 0.03).max(cap_mark_y()) }

// A point on the strand between you (a) and them (b), t from 0 to 1, at
// hand height with a little sag, lifted by `lift` metres, in the world.
fn strand_point(a, b, t, lift) {
    let y = a[1] + (b[1] - a[1]) * t + strand_height() - 0.25 * (PI() * t).sin() + lift;
    [a[0] + (b[0] - a[0]) * t, y, a[2] + (b[2] - a[2]) * t]
}

// The same point on the screen.
fn strand_at(a, b, t, lift) {
    let p = strand_point(a, b, t, lift);
    world_to_screen(p[0], p[1], p[2])
}

// The strand between you and whoever you're trading with. A request is a
// faint line with light flowing from the asker; an open trade is two
// fibres twisting round each other, blue, each end gold once its side has
// accepted.
fn draw_strand(you, trade, now) {
    let a = [you[0], you[1], you[2]];
    let b = [trade.x, trade.y, trade.z];
    let len = dist(a[0], a[2], b[0], b[2]);
    if len < 0.5 { return; }
    let blue = trade_blue();
    let steps = 28;
    if trade.state != "open" {
        // From the asker: you when asking, them when they've asked you.
        let dir = if trade.state == "asking" { 1.0 } else { -1.0 };
        let prev = strand_point(a, b, 0.0, 0.0);
        for i in 1..=steps {
            let t = i.to_float() / steps.to_float();
            let p = strand_point(a, b, t, 0.0);
            let flow = ((t * 3.0 - dir * now * 0.8) % 1.0 + 1.0) % 1.0;
            let glow = 0.12 + 0.35 * smooth(1.0 - (flow - 0.5).abs() * 2.0);
            draw_world_line(prev[0], prev[1], prev[2], p[0], p[1], p[2],
                            blue[0], blue[1], blue[2], glow, 1.5);
            prev = p;
        }
        return;
    }
    let g = gold();
    let turns = len * 1.2;
    for k in 0..2 {
        let phase = k.to_float() * PI();
        let prev = ();
        for i in 0..=steps {
            let t = i.to_float() / steps.to_float();
            let twist = 0.06 * (t * turns * 2.0 * PI() + phase + now * 1.4).sin();
            let p = strand_point(a, b, t, twist);
            if prev != () {
                // Your end golds as you accept, theirs as they do.
                let mine = if trade.mine { smooth((0.55 - t) / 0.3) } else { 0.0 };
                let theirs = if trade.theirs { smooth((t - 0.45) / 0.3) } else { 0.0 };
                let w = mine.max(theirs);
                let c = [blue[0] + (g[0] - blue[0]) * w,
                         blue[1] + (g[1] - blue[1]) * w,
                         blue[2] + (g[2] - blue[2]) * w];
                draw_world_line(prev[0], prev[1], prev[2], p[0], p[1], p[2],
                                c[0], c[1], c[2], 0.75, 1.5);
            }
            prev = p;
        }
    }
}

// A trade made: a gold pulse running out from the middle of the strand to
// each of you, and a ring breathing out round each.
fn draw_traded(d, you, now) {
    let k = (now - d.born) / trade_life();
    let g = gold();
    let feet = if you != () { you[1] } else { cap_ground_y() };
    let ease = 1.0 - (1.0 - k) * (1.0 - k);
    for end in [[d.x, d.z], [d.tx, d.tz]] {
        ring(end[0], end[1], feet_mark(feet), 0.5 + 1.8 * ease, g, 0.85 * (1.0 - k), 2.0, 44);
    }
    if k > 0.5 { return; }
    let a = [d.x, feet, d.z];
    let b = [d.tx, feet, d.tz];
    let reach = smooth(k / 0.5) * 0.5;
    for side in [-1.0, 1.0] {
        let t = 0.5 + side * reach;
        let p = strand_at(a, b, t, 0.0);
        if p.visible { draw_circle(p.x, p.y, 4.0, g[0], g[1], g[2], 0.95 * (1.0 - k * 2.0)); }
    }
}

// Pale stone, for a lot that isn't yours.
fn stone() { [0.80, 0.78, 0.72] }

// The picked lot's edge: a fine line round it, and a brighter stretch a
// fifth of the way round, running round it every few seconds. A claim
// just made flares the whole edge as its rings go out.
fn draw_plot_edge(p, now, flare) {
    let c = if p.mine { gold() } else { stone() };
    let pts = p.outline;
    let n = pts.len();
    if n < 2 { return; }
    // The perimeter, so the bright stretch runs at an even speed.
    let lens = [];
    let total = 0.0;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        let l = ((b[0] - a[0]) * (b[0] - a[0]) + (b[1] - a[1]) * (b[1] - a[1])).sqrt();
        lens.push(l);
        total += l;
    }
    if total <= 0.0 { return; }
    let head = (now / 4.0 % 1.0) * total;
    let run = total * 0.2;
    let base = 0.45 + 0.5 * flare;
    let width = 1.5 + 1.5 * flare;
    let along = 0.0;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        // In short steps, so the stretch can brighten part of an edge.
        let steps = (lens[i] / 0.5).to_int().max(1);
        for k in 0..steps {
            let t0 = k.to_float() / steps.to_float();
            let t1 = (k + 1).to_float() / steps.to_float();
            let mid = along + lens[i] * (t0 + t1) / 2.0;
            let behind = (head - mid + total) % total;
            let lit = if behind < run { 1.0 - behind / run } else { 0.0 };
            draw_world_line(a[0] + (b[0] - a[0]) * t0, p.y, a[1] + (b[1] - a[1]) * t0,
                            a[0] + (b[0] - a[0]) * t1, p.y, a[1] + (b[1] - a[1]) * t1,
                            c[0], c[1], c[2], (base + 0.5 * lit).min(1.0), width + lit);
        }
        along += lens[i];
    }
}

// A plot claimed: two gold rings breathing out from its middle to past
// its edge, the second a beat behind and fainter.
fn draw_deed(d, now) {
    let k = (now - d.born) / deed_life();
    let y = cap_ground_y() + 0.2;
    for i in 0..2 {
        let a = (k - i.to_float() * 0.18) / 0.82;
        if a <= 0.0 || a >= 1.0 { continue; }
        let ease = 1.0 - (1.0 - a) * (1.0 - a);
        let alpha = (if i == 0 { 0.9 } else { 0.45 }) * (1.0 - a);
        ring(d.x, d.z, y, 0.4 + d.r * 1.25 * ease, gold(), alpha, 2.0, 64);
    }
}

// A tuft grown back: a green ring breathing out over the ground.
fn draw_sprout(s, now) {
    let k = (now - s.born) / sprout_life();
    let ease = 1.0 - (1.0 - k) * (1.0 - k);
    let c = [0.42, 0.64, 0.47];
    ring(s.x, s.z, cap_mark_y(), 0.4 + 1.6 * ease, c, 0.7 * (1.0 - k), 1.5, 40);
}

// Two gold rings spreading over the ground from where the click landed,
// the second a beat behind, both easing out as they fade.
fn draw_ripple(rip, now) {
    for k in 0..2 {
        let age = (now - rip.born) / ripple_life() - k.to_float() * 0.2;
        if age <= 0.0 || age >= 1.0 { continue; }
        let ease = 1.0 - (1.0 - age) * (1.0 - age);
        let alpha = 0.85 * (1.0 - age) * (1.0 - age);
        ring(rip.x, rip.z, cap_mark_y(), 0.3 + 5.5 * ease, gold(), alpha, 1.5, 48);
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

// The reticle: a thin gold ring lying on the ground under the cursor, a
// fainter one outside it, and four ticks pointing north, east, south and
// west, so the ground's own perspective shows in its shape.
fn draw_reticle(g, now) {
    let breathe = 1.0 + 0.06 * (now * 2.4).sin();
    let r = 0.9 * breathe;
    let c = gold();
    let y = cap_mark_y();
    ring(g[0], g[1], y, r, c, 0.9, 1.5, 40);
    ring(g[0], g[1], y, r * 1.9, c, 0.25, 1.0, 56);
    for a in [0.0, 0.5 * PI(), PI(), 1.5 * PI()] {
        draw_world_line(g[0] + r * 1.25 * a.cos(), y, g[1] + r * 1.25 * a.sin(),
                        g[0] + r * 1.7 * a.cos(), y, g[1] + r * 1.7 * a.sin(),
                        c[0], c[1], c[2], 0.7, 1.5);
    }
}

fn smooth(t) {
    let t = t.max(0.0).min(1.0);
    t * t * (3.0 - 2.0 * t)
}
