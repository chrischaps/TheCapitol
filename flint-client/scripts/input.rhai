// Input to intents (plan §4.2): what a left click on the world means, and
// the marks it leaves. P4.3 is click to move, P4.4 gathering, P6.3 stations.
//
// A click is picked off the ground (`pick`: the land where it has risen,
// W9, else the flat ground) and offered to each intent in the React client's order (GameCanvas.tsx handleClick).
// The first that takes it wins:
//
//   1. placing a station: put it here              (P6.2)
//   2. shift-click a player: trade, or walk closer  (P7.3)
//      click a player: their card, who they are     (S0.3, here)
//   3. a station: open it, or walk closer           (P6.3)
//      the Gazette's notice board: read it         (E3.9, here)
//      the Deeds Office's desk: look in            (#448, here)
//      a bazaar stall: its shop, or walk there      (E3.11, here)
//      the Exchange hall: the market, or walk there (E3.7, here)
//   4. alt-click a plot: select it                  (P6.4, here)
//   5. an available resource node: extract          (P4.4, here)
//   6. anywhere else: walk there                    (P4.3, here)
//      but not over the shoulder, where WASD walks  (T1.3)
//
// Over the shoulder (T1.4) the cursor is captured while nothing wants it
// (camera.rhai), and then it isn't a pointer: there are no hover rings and
// a click doesn't go down the list above. It's interact, which acts on the
// thing in front of you as E does (T1.5): `cap_focus()` says what, and
// draw_focus rings it and says what E will do. A dot at the screen's centre
// is where you're looking (T1.6, draw_aim): the town's gold on a dark edge,
// faint with nothing in focus and full with something. With the cursor freed (a panel, Alt)
// clicks go down the list as in Click mode, but for the walk. Once Esc has
// let the cursor go, a click on the world only takes it back.
//
// Each is a function below that gets the click (#{ x, z } in world
// metres, `shift`, `alt`) and returns true when it took it.
//
// Walking: cap_move(x, z) sends you (clamped to the world disc, r6800 server
// units), and cap_destination() is where you're going until you arrive.
// A gold ring lies there, with a faint dashed line marching to it from
// your feet. The click itself drops a ripple, as the spike's did (P0.4).
// A walk across the water goes round by a bridge or a ford (#267): Rust
// plans it on the walk graph and sends it a leg at a time, and
// cap_route() is the corners still to pass, so the dashes run round them,
// with a faint pip at each turn.
//
// The server stops a walk at the water's edge, or below a cliff (W4), with
// `movement_blocked`. Rust fires it here with where you stand, which way
// you were going, and `steep` for a cliff. The water splashes just ahead
// of you, the camera rig gives a small nudge (camera.rhai), and a toast
// says to find a bridge or a ford, or a gentler way up (notices.rhai,
// which holds every toast).
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
// The Exchange hall (E3.7). A click anywhere on it (cap_hall_at, tried at
// heights up its walls, so the clock and the roof take the click as well as
// the ground) opens the market panel there, or walks you to its door and
// opens it as you arrive (cap_visit_hall; capitol_core::hall). Hovering it,
// or standing in range, traces its footprint in the Trade District's blue,
// and a prompt hangs over it: what a click will do, unless the market is
// open already. While goods wait in your market box, gold motes rise from
// its lantern, a slow fountain you can find from across the city.
//
// The bazaar (E3.11), across the avenue from the hall. A click on a stall
// (cap_stall_at, tried down from its eaves, so the roof and the awning take
// the click; the lane between the rows stays ground to walk on) opens its
// shop there, or walks you along the bazaar's streets and opens it as
// you reach the counter (cap_visit_stall; capitol_core::bazaar). Each kept
// stall's name is painted on both faces of the sign hanging from its
// awning, while the camera is near enough to read it. It's world text
// (draw_world_text, #274), so a roof in front of it hides it. Hovering a stall, or standing in its
// range, traces its footprint in the lanterns' amber, and the one you're
// walking to breathes; the one under the cursor says whose it is and what
// a click does.
//
// The Gazette's notice board (E3.9) stands in the hall's forecourt, on the
// side nearer the tower. A click on it (cap_board_at, tried up its posts)
// opens the paper (cap_read_gazette) wherever you are: reading needs no
// walk. Hovering it names it, and while an edition is out you haven't
// read, a gold glint breathes over its lamp.
//
// The Deeds Office (#448) is the desk beside it, under the wax-red awning.
// A click on it (cap_office_at, tried up its posts) opens the office's
// panel (cap_open_office) wherever you are: the office explains the deed
// and compares the zones, and is never a toll (plan D8). Hovering it names
// it, and while your deed waits there, earned and unspent, a gold glint
// hangs over its awning.
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
// The road in (#442): where the card's step leads (cap_lead: the nearest
// tuft or tree of its kind, a lot, the hall's door, a stall's counter) is
// ringed in gold on the ground, slowly, with a glint over it, as the notice
// board's over its lamp, and a few motes rising, as from the hall. A lot's
// edge is traced with it. Off screen, the card's arrow points the way
// instead (ui/hints.rs).
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
// The aim dot's brightness, 0 faint to 1 full, easing toward the focus's,
// and when it was last drawn (T1.6).
let aim = #{ k: 0.0, at: () };

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
        let h = pick();
        if h == () { cap_place_off(); } else { cap_place_hover(h[0], h[1]); }
    }
    if !is_mouse_just_pressed(0) { return; }
    // Over the shoulder (T1.4): a captured click is interact (T1.5), which
    // acts on what's in front of you, as E does; a click after Esc let the
    // cursor go takes it back
    if is_cursor_captured() { cap_interact(); return; }
    if cap_cursor_let_go() { cap_take_cursor(); return; }

    // The ground under the cursor, or () over the sky.
    let g = pick();
    if g == () { return; }
    let click = #{
        x: g[0], z: g[1],
        shift: is_action_pressed("with_shift"),
        alt: is_action_pressed("with_alt"),
    };
    if place_station(click) { return; }
    if trade_with(click) { return; }
    if open_card(click) { return; }
    if open_station(click) { return; }
    if read_gazette() { return; }
    if open_office() { return; }
    if visit_stall() { return; }
    if visit_hall() { return; }
    if select_plot(click) { return; }
    if extract(click) { return; }
    if cap_setting("control_mode") == "direct" { return; }
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
// A plain click on someone: their card (S0.3). Shift asked to trade above.
fn open_card(click) {
    let p = cap_player_at(click.x, click.z);
    if p == () { return false; }
    cap_open_card(p.id);
    true
}
fn open_station(click) {           // P6.3
    let s = cap_station_at(click.x, click.z);
    if s == () { return false; }
    cap_open_station(s.id);
    true
}

// The Exchange hall under the cursor: the market, there or after a walk.
fn visit_hall() {                   // E3.7
    if !hall_under_cursor() { return false; }
    cap_visit_hall();
    true
}

// A stall in the bazaar under the cursor: its shop, there or after a walk.
fn visit_stall() {                  // E3.11
    let id = stall_under_cursor();
    if id == () { return false; }
    cap_visit_stall(id);
    true
}

// The stall the cursor's ray meets first: its point at each height from
// the eaves down, on a stall's footprint. The camera is above, so the
// highest hit is the nearest. The lane between the rows is no stall's.
fn stall_under_cursor() {
    if cap_bazaar() == () { return (); }
    let y = cap_ground_y();
    for k in 0..10 {
        let p = screen_to_ground(mouse_x(), mouse_y(), y + 2.7 - 0.3 * k.to_float());
        if p != () {
            let id = cap_stall_at(p[0], p[1]);
            if id != () { return id; }
        }
    }
    ()
}

// The notice board under the cursor: open it, on Wanted or the Gazette.
fn read_gazette() {                 // E3.9
    if !board_under_cursor() { return false; }
    cap_read_gazette();
    true
}

// The Deeds Office's desk under the cursor: look in.
fn open_office() {                  // #448
    if !office_under_cursor() { return false; }
    cap_open_office();
    true
}

// Whether the cursor's ray meets the desk: its point at each height up its
// posts, on its slot.
fn office_under_cursor() {
    if cap_hall() == () { return false; }
    let y = cap_ground_y();
    for k in 0..10 {
        let p = screen_to_ground(mouse_x(), mouse_y(), y + 0.25 * k.to_float());
        if p != () && cap_office_at(p[0], p[1]) { return true; }
    }
    false
}

// Whether the cursor's ray meets the board: its point at each height up
// its posts, on its slot.
fn board_under_cursor() {
    if cap_hall() == () { return false; }
    let y = cap_ground_y();
    for k in 0..10 {
        let p = screen_to_ground(mouse_x(), mouse_y(), y + 0.25 * k.to_float());
        if p != () && cap_board_at(p[0], p[1]) { return true; }
    }
    false
}

// Whether the cursor's ray meets the hall: its point at each height up to
// the roof, on the footprint. That's the hall's box, near enough.
fn hall_under_cursor() {
    if cap_hall() == () { return false; }
    let y = cap_ground_y();
    for k in 0..9 {
        let p = screen_to_ground(mouse_x(), mouse_y(), y + k.to_float());
        if p != () && cap_hall_at(p[0], p[1]) { return true; }
    }
    false
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
            x: data.x, z: data.z, born: t, base: cap_ground_at(data.x, data.z) + 1.3, up: 0.0,
            text: "+" + data.quantity + " " + data.item_name,
            grade: data.grade, colour: data.colour,
        });
        return;
    }
    if name == "crafting_completed" {
        let you = cap_follow();
        let feet = if you != () { you[1] } else { cap_ground_at(data.x, data.z) };
        yields.push(#{
            x: data.x, z: data.z, born: t, base: feet + name_height(), up: craft_lift(),
            text: "+" + data.quantity + " " + data.item_name,
            grade: data.grade, colour: data.colour,
        });
        blooms.push(#{ x: data.x, z: data.z, y: feet_mark(data.x, data.z, feet), born: t });
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
    // A cliff stops you dry: only the destination marker goes.
    if data.steep == true {
        if marker != () && marker.gone == () { marker.gone = t; }
        return;
    }
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
    // A captured cursor (T1.4) hovers nothing: it's a view, not a pointer.
    let aiming = is_cursor_captured();
    let hover = if aiming { () } else { pick() };
    // What E acts on over the shoulder (T1.5): it stands in for the hover
    let focus = if aiming { cap_focus() } else { () };
    let placing = cap_placing();
    if placing != () { draw_placement(placing, now); }
    // While placing, a station under the cursor is in the ghost's way, not
    // a thing to open.
    let over = if hover != () && placing == () { cap_station_at(hover[0], hover[1]) } else { () };
    for s in cap_stations() {
        // One under the cursor shows its ring, faintly if it's out of reach.
        if over != () && over.id == s.id && s.reach < hover_reach() { s.reach = hover_reach(); }
        if s.reach > 0.01 { draw_station_ring(s, now); }
        let hovered = over != () && over.id == s.id;
        if s.mine { draw_station_mine(s, hovered, now); }
        if s.paused != () { draw_station_ember(s, hovered, now); }
        if s.worn != () { draw_station_worn(s, hovered, now); }
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
        if you != () { draw_route(you[0], you[2], route_to(target.x, target.z), now, 0.6); }
        draw_sight(target, now);
    } else if marker != () {
        if you != () && marker.gone == () {
            draw_route(you[0], you[2], route_to(marker.x, marker.z), now, 0.38);
        }
        draw_marker(marker, now);
    }
    for c in cuttings { draw_cutting(c, now); }
    for s in sprouts { draw_sprout(s, now); }
    for y in yields { draw_yield(y, now); }

    for rip in ripples { draw_ripple(rip, now); }
    for s in splashes { draw_splash(s, now); }

    let hall = cap_hall();
    let free = hall != () && placing == () && over == () && !aiming;
    let over_board = (free && board_under_cursor()) || focus_is(focus, "board");
    let over_office = (free && !over_board && office_under_cursor()) || focus_is(focus, "office");
    let over_hall = (free && !over_board && !over_office && hall_under_cursor()) || focus_is(focus, "hall");
    if hall != () {
        draw_hall(hall, over_hall, now);
        draw_board(hall, over_board, now);
        draw_office(hall, over_office, now);
    }
    let bazaar = cap_bazaar();
    let over_stall = if bazaar != () && placing == () && over == () && !aiming && !over_board && !over_office && !over_hall {
        stall_under_cursor()
    } else if focus_is(focus, "stall") { focus.id } else { () };
    if bazaar != () { draw_bazaar(bazaar, over_stall, now); }
    let lead = cap_lead();
    if lead != () && !cap_clean() { draw_lead(lead, now); }
    if focus != () && placing == () { draw_focus(focus, now); }
    if aiming {
        // The aim dot eases toward full while something's in focus
        let dt = if aim.at == () { 1.0 } else { (now - aim.at).max(0.0) };
        let want = if focus != () && placing == () { 1.0 } else { 0.0 };
        aim.k += (want - aim.k) * (1.0 - (-dt * 14.0).exp());
        aim.at = now;
        draw_aim(aim.k);
    } else {
        aim.at = ();
    }

    if hover != () && placing == () && over == () && !over_hall && !over_board && !over_office && over_stall == () {
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
    let at = world_to_screen(p.x, y + cap_rise(p.x, p.z), p.z);
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
    // Where it would stand and the room left there (V3.5): indoors,
    // upstairs, the yard or open ground, in the gold of what's yours
    let next = top + m.height + 4.0;
    if p.place != () {
        let pm = measure_text_ex(p.place, 13.0, "body", 0.0);
        draw_text_opts(at.x - pm.width / 2.0 + 1.0, next + 1.0, p.place, 13.0, #{
            font: "body", color: [0.0, 0.0, 0.0, 0.55],
        });
        draw_text_opts(at.x - pm.width / 2.0, next, p.place, 13.0, #{
            font: "body", color: [0.95, 0.84, 0.55, 0.95],
        });
        next += pm.height + 3.0;
    }
    if p.storey != () {
        let sm = measure_text_ex(p.storey, 12.0, "body", 0.0);
        draw_text_opts(at.x - sm.width / 2.0, next, p.storey, 12.0, #{
            font: "body", color: [0.95, 0.84, 0.55, 0.65],
        });
        next += sm.height + 3.0;
    }
    let hint = p.kit + " · right-click or Esc to stop";
    let hm = measure_text_ex(hint, 12.0, "body", 0.0);
    draw_text_opts(at.x - hm.width / 2.0, next, hint, 12.0, #{
        font: "body", color: [0.88, 0.88, 0.88, 0.6],
    });
}

fn dist(ax, az, bx, bz) {
    let dx = ax - bx;
    let dz = az - bz;
    (dx * dx + dz * dz).sqrt()
}

// Marks on the ground are world lines (#86): drawn in the 3D frame, so
// whatever stands in front of them hides them. Lay them at cap_mark_y(),
// through mark_line, which lifts each end onto the land (W9).

// The ground under the cursor as [x, z], or () over the sky: the land's
// height field where it's in (W8, W9), else the flat ground.
fn pick() {
    let h = screen_to_heightfield(mouse_x(), mouse_y());
    if h != () { return [h[0], h[2]]; }
    screen_to_ground(mouse_x(), mouse_y(), cap_ground_y())
}

// A world line from (ax, ay, az) to (bx, by, bz), each end lifted by the
// land under it, so a mark at cap_mark_y() lies on a hill as on the street.
// Keep its segments short (a metre or two) where the land is steep.
fn mark_line(ax, ay, az, bx, by, bz, c, alpha, width) {
    draw_world_line(ax, ay + cap_rise(ax, az), az, bx, by + cap_rise(bx, bz), bz,
                    c[0], c[1], c[2], alpha, width);
}

// A straight line along the ground from (ax, az) to (bx, bz).
fn ground_line(ax, az, bx, bz, y, c, alpha, width) {
    mark_line(ax, y, az, bx, y, bz, c, alpha, width);
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

// An ember floating over a station of yours that's waiting (E4.4): a
// small flame-shaped mote that bobs and flickers, a few points wide however
// far away you stand, and on hover the reason under it. Its light is the
// world's (stations.rs); this only says "this one needs you".
fn draw_station_ember(s, over, now) {
    let seed = s.x * 1.7 + s.z * 0.9;
    let bob = 0.08 * (now * 1.3 + seed).sin();
    let p = world_to_screen(s.x, cap_ground_at(s.x, s.z) + s.top + 0.55 + bob, s.z);
    if !p.visible { return; }
    // Over your gold mark, which it always has: only yours have embers.
    if s.mine { p.y -= mine_mark_height() + 12.0; }
    let flicker = 0.8 + 0.2 * (now * 7.3 + seed).sin() * (now * 3.1).cos();
    let r = 4.2 * flicker;
    let hot = station_orange();
    // A soft halo, then the flame: a drop with its point up, and a pale heart
    draw_circle(p.x, p.y - r * 0.4, r * 2.4, hot[0], hot[1], hot[2], 0.07 * flicker);
    draw_polygon([[p.x, p.y - r * 2.0], [p.x + r, p.y], [p.x, p.y + r * 0.9], [p.x - r, p.y]],
        hot[0], hot[1], hot[2], 0.92, 2);
    draw_circle(p.x, p.y + r * 0.1, r * 0.45, 1.0, 0.85, 0.5, 0.95);
    if !over { return; }
    let line = s.paused;
    let size = 13.0;
    let m = measure_text_ex(line, size, "strong", 0.0);
    let y = p.y + r * 2.0 + 6.0;
    draw_text_opts(p.x - m.width / 2.0 + 1.0, y + 1.0, line, size, #{
        font: "strong", color: [0.0, 0.0, 0.0, 0.6],
    });
    draw_text_opts(p.x - m.width / 2.0, y, line, size, #{
        font: "strong", color: [1.0, 0.77, 0.42, 0.95],
    });
}

// The gold over a station of yours (#487), so yours reads as yours in a
// crowd of the same, as at the spawn after Arrival's bench step: a small
// lozenge in the Capitol's gold, the plot panel's and your card's "yours",
// its point down at the station, hanging still but for a slow breath. It
// keeps a few points' size however far you stand, and fades out past a
// street or two, so a yard full of your own doesn't wear a field of them.
// A station worn by the weather to needing repair (V3.4): a small rusted
// tag beside it, a diamond of oxide with a dark rim and a pale crack down
// it, swaying a little as if hung on a nail. It fades with distance as
// the gold does, and on hover it says so, with what's left of it.
fn draw_station_worn(s, over, now) {
    let cam = get_camera_position();
    let d = dist(s.x, s.z, cam.x, cam.z);
    let fade = ((mine_mark_far() - d) / (mine_mark_far() - mine_mark_near())).max(0.0).min(1.0);
    fade *= (s.top * 2.0).min(1.0);
    if fade <= 0.01 { return; }
    let p = world_to_screen(s.x, cap_ground_at(s.x, s.z) + s.top * 0.6, s.z);
    if !p.visible { return; }
    // Beside the station, clear of the gold and the ember over it.
    let seed = s.x * 0.9 + s.z * 1.3;
    let sway = 0.12 * (now * 1.7 + seed).sin();
    let r = if over { 7.5 } else { 6.0 };
    let cx = p.x + 18.0;
    let cy = p.y;
    let q = 1.570796;
    let rim = [];
    let tag = [];
    for i in 0..4 {
        rim.push(worn_point(cx, cy, q * i.to_float() + sway, r * 1.25));
        tag.push(worn_point(cx, cy, q * i.to_float() + sway, r));
    }
    draw_polygon(rim, 0.13, 0.08, 0.05, 0.75 * fade, 2);
    draw_polygon(tag, 0.69, 0.33, 0.16, 0.95 * fade, 2);
    let a = worn_point(cx, cy, sway, r * 0.55);
    let b = worn_point(cx, cy, q * 2.0 + sway, r * 0.15);
    draw_line(a[0], a[1], b[0], b[1], 0.98, 0.86, 0.7, 0.95 * fade, 1.6);
    let dot = worn_point(cx, cy, q * 2.0 + sway, r * 0.5);
    draw_circle(dot[0], dot[1], 1.1, 0.98, 0.86, 0.7, 0.95 * fade);
    if !over { return; }
    let line = "Needs repair (" + s.worn + "%)";
    let size = 13.0;
    let m = measure_text_ex(line, size, "strong", 0.0);
    let y = cy + r * 1.6 + 6.0;
    draw_text_opts(cx - m.width / 2.0 + 1.0, y + 1.0, line, size, #{
        font: "strong", color: [0.0, 0.0, 0.0, 0.6],
    });
    draw_text_opts(cx - m.width / 2.0, y, line, size, #{
        font: "strong", color: [0.95, 0.62, 0.42, 0.95],
    });
}

// A point `r` out from (cx, cy) at angle `a`, 0 straight up.
fn worn_point(cx, cy, a, r) { [cx + a.sin() * r, cy - a.cos() * r] }

fn mine_mark_height() { 13.0 }
fn mine_mark_near() { 70.0 }
fn mine_mark_far() { 160.0 }

fn draw_station_mine(s, over, now) {
    let cam = get_camera_position();
    let d = dist(s.x, s.z, cam.x, cam.z);
    let fade = ((mine_mark_far() - d) / (mine_mark_far() - mine_mark_near())).max(0.0).min(1.0);
    // It comes in with the station as it rises or drops into place.
    fade *= (s.top * 2.0).min(1.0);
    if fade <= 0.01 { return; }
    let ground = cap_ground_at(s.x, s.z);
    let p = world_to_screen(s.x, ground + s.top + 0.5, s.z);
    if !p.visible { return; }
    // A fine stem down to the station's top, like a surveyor's pin, so up
    // close the mark can't be read as the one behind's.
    let foot = world_to_screen(s.x, ground + s.top + 0.05, s.z);
    let seed = s.x * 1.3 + s.z * 0.7;
    let breath = 0.5 + 0.5 * (now * 1.1 + seed).sin();
    let g = gold();
    let h = if over { mine_mark_height() * 1.15 } else { mine_mark_height() };
    let w = h * 0.42;
    let tip = p.y - 2.0;
    let mid = tip - h * 0.62;
    let top = tip - h;
    let a = if over { fade } else { fade * 0.9 };
    // A soft halo, breathing, then the lozenge with a dark rim so it holds
    // against a bright sky or a sunlit street, and a pale facet up its left.
    if foot.visible && foot.y - tip > 3.0 {
        draw_line(p.x, tip, foot.x, foot.y, g[0], g[1], g[2], a * 0.55, 1.2);
    }
    draw_circle(p.x, mid, h * (0.62 + 0.08 * breath), g[0], g[1], g[2], a * (0.05 + 0.05 * breath));
    let shape = [[p.x, top], [p.x + w, mid], [p.x, tip], [p.x - w, mid]];
    draw_polygon(shape, g[0], g[1], g[2], a, 2);
    draw_polygon([[p.x, top], [p.x, tip], [p.x - w, mid]], 1.0, 0.9, 0.55, a * 0.45, 2);
    draw_polygon_outline(shape, 0.24, 0.17, 0.03, a * 0.75, 1.2, true, 2);
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
        mark_line(px, y, pz, qx, y, qz, c, alpha, width);
        px = qx;
        pz = qz;
    }
}

// The way you're walking: dashes from your feet to the ring, marching
// toward it, faint, and fading in from under you so they don't cross the
// avatar. Long walks get longer dashes, so the count stays bounded.
// The corners of the walk to (x, z): cap_route()'s, ending at (x, z)
// itself, which is what's marked there.
fn route_to(x, z) {
    let corners = cap_route();
    if corners.len() > 0 { corners.pop(); }
    corners.push([x, z]);
    corners
}

// The dashes from your feet through each corner to the end, as one line:
// they march on round a corner rather than starting again at it.
fn draw_route(x0, z0, corners, now, strength) {
    let legs = [];
    let total = 0.0;
    let ax = x0;
    let az = z0;
    for p in corners {
        let dx = p[0] - ax;
        let dz = p[1] - az;
        let span = (dx * dx + dz * dz).sqrt();
        if span > 0.001 {
            legs.push(#{ x: ax, z: az, ux: dx / span, uz: dz / span, at: total, span: span });
            total += span;
        }
        ax = p[0];
        az = p[1];
    }
    if total < 1.5 { return; }
    let step = (total / 90.0).max(1.4);
    let dash = step * 0.55;
    let y = cap_mark_y();
    let c = gold();
    let s = (now * 1.6 % 1.0) * step;
    // Stop short of the ring, start clear of your feet.
    let end = total - 1.3;
    let i = 0;
    while s < end {
        let a = s.max(0.8);
        let b = (s + dash).min(end);
        if b > a {
            let fade = ((a - 0.8) / 2.5).min(1.0);
            while i < legs.len() - 1 && legs[i].at + legs[i].span <= a { i += 1; }
            // A dash over a corner is drawn a piece a leg
            let j = i;
            let lo = a;
            while j < legs.len() && lo < b {
                let l = legs[j];
                let hi = b.min(l.at + l.span);
                let t0 = lo - l.at;
                let t1 = hi - l.at;
                mark_line(l.x + l.ux * t0, y, l.z + l.uz * t0, l.x + l.ux * t1, y, l.z + l.uz * t1,
                          c, strength * fade, 1.5);
                lo = hi;
                j += 1;
            }
        }
        s += step;
    }
    // A pip at each turn still to come, breathing with the ring.
    let breathe = 1.0 + 0.08 * (now * 3.0).sin();
    for k in 1..legs.len() {
        ring(legs[k].x, legs[k].z, y, 0.35 * breathe, c, strength * 0.8, 1.5, 12);
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
        let g = cap_ground_at(c.x, c.z);
        let y = g + 0.3 + b.vy * t - 4.9 * t * t;
        if y < g { continue; }
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
    let y = feet_mark(p.x, p.z, p.feet_y);
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

// ── The Exchange hall ────────────────────────────────────────────────────

// How often a mote rises from the hall while goods wait (s), how long each
// takes, and how high it goes over the roof (m).
fn mote_every() { 0.8 }
fn mote_life() { 3.2 }
fn mote_rise() { 11.0 }

// ── Focus (T1.5) ─────────────────────────────────────────────────────────

// Whether the focus is a `kind`.
fn focus_is(f, kind) { f != () && f.kind == kind }

// A prompt's "click" as E over the shoulder (T1.5), where the cursor is
// captured and E, or a click, acts on the focus; or as A, with a pad in
// your hands (T2.2), which `draw_act_line` draws as the button.
fn act_words(line) {
    if is_cursor_captured() {
        let key = if pad_held() { "{A}" } else { "E" };
        line.replace("Click", "Press " + key);
        line.replace("click", "press " + key);
    }
    line
}

// The pad is in your hands over the shoulder (T2.2): it was the last thing
// used, with the cursor captured. The prompts then show its buttons.
fn pad_held() { is_cursor_captured() && last_input_device() == "gamepad" }

// A face button's tint, as the shortcut card's (`theme::pad_tint`): sage
// A, brick B, slate X, ochre Y. `()` for the rest.
fn pad_tint(name) {
    switch name {
        "A" => [0.561, 0.749, 0.478],
        "B" => [0.831, 0.478, 0.373],
        "X" => [0.498, 0.624, 0.812],
        "Y" => [0.890, 0.757, 0.353],
        _ => (),
    }
}

// One of the pad's controls as a glyph `h` high with its top left at x, y,
// drawn rather than set in a font (T2.2), as the shortcut card draws them:
// a face button round, ringed and lettered in its tint; the d-pad ("Up",
// "Down", "Left", "Right") a cross with that arm lit gold; anything else a
// small cap with its name. Returns how wide it was.
fn draw_pad_glyph(x, y, name, h, a) {
    let r = h / 2.0;
    let cx = x + r;
    let cy = y + r;
    let tint = pad_tint(name);
    if tint != () {
        draw_circle(cx + 0.5, cy + 1.5, r, 0.0, 0.0, 0.0, 0.35 * a);
        draw_circle(cx, cy, r - 0.5, 0.10, 0.09, 0.07, 0.86 * a);
        draw_circle_outline(cx, cy, r - 1.0, tint[0], tint[1], tint[2], 0.95 * a, 1.5);
        let m = measure_text_ex(name, h * 0.62, "strong", 0.0);
        draw_text_opts(cx - m.width / 2.0, cy - m.height / 2.0, name, h * 0.62, #{
            font: "strong", color: [tint[0], tint[1], tint[2], a],
        });
        return h;
    }
    let way = switch name {
        "Up" => [0.0, -1.0],
        "Down" => [0.0, 1.0],
        "Left" => [-1.0, 0.0],
        "Right" => [1.0, 0.0],
        _ => (),
    };
    if way != () {
        let arm = (h * 0.36).round();
        draw_rect_ex(x + 0.5, y + r - arm / 2.0 + 1.5, h, arm, 0.0, 0.0, 0.0, 0.35 * a, 2.0, 0);
        draw_rect_ex(x, y + r - arm / 2.0, h, arm, 0.10, 0.09, 0.07, 0.86 * a, 2.0, 0);
        draw_rect_ex(cx - arm / 2.0, y, arm, h, 0.10, 0.09, 0.07, 0.86 * a, 2.0, 0);
        draw_rect_outline_ex(x, y + r - arm / 2.0, h, arm, 0.79, 0.64, 0.15, 0.45 * a, 1.0, 2.0, 0);
        draw_rect_outline_ex(cx - arm / 2.0, y, arm, h, 0.79, 0.64, 0.15, 0.45 * a, 1.0, 2.0, 0);
        draw_rect_ex(cx - arm / 2.0 + 1.0, cy - arm / 2.0 + 1.0, arm - 2.0, arm - 2.0, 0.10, 0.09, 0.07, a, 0.0, 0);
        let lit = arm - 3.0;
        let reach = r - arm / 2.0;
        let c = gold();
        draw_rect_ex(cx + way[0] * reach - lit / 2.0, cy + way[1] * reach - lit / 2.0, lit, lit,
                     c[0], c[1], c[2], a, 1.5, 0);
        return h;
    }
    let m = measure_text_ex(name, h * 0.6, "strong", 0.0);
    let w = (m.width + 10.0).max(h);
    draw_rect_ex(x, y, w, h, 0.10, 0.09, 0.07, 0.86 * a, h / 2.0, 0);
    draw_rect_outline_ex(x, y, w, h, 0.79, 0.64, 0.15, 0.8 * a, 1.0, h / 2.0, 0);
    draw_text_opts(x + (w - m.width) / 2.0, y + (h - m.height) / 2.0, name, h * 0.6, #{
        font: "strong", color: [0.96, 0.90, 0.72, a],
    });
    w
}

// A prompt's second line, centred on `cx` with its shadow, in `ink` at
// `alpha`: the pad's buttons in it ("{A}", act_words's) drawn as glyphs.
fn draw_act_line(cx, y, line, size, ink, alpha) {
    let pieces = [];
    let rest = line;
    loop {
        let open = rest.index_of("{");
        let close = rest.index_of("}");
        if open < 0 || close < open {
            if rest != "" { pieces.push([false, rest]); }
            break;
        }
        if open > 0 { pieces.push([false, rest.sub_string(0, open)]); }
        pieces.push([true, rest.sub_string(open + 1, close - open - 1)]);
        rest = rest.sub_string(close + 1);
    }
    let h = size + 5.0;
    let gap = 3.0;
    let width = 0.0;
    let tall = 0.0;
    for p in pieces {
        if p[0] {
            width += draw_pad_glyph_width(p[1], h) + 2.0 * gap;
        } else {
            let m = measure_text_ex(p[1], size, "strong", 0.0);
            width += m.width;
            tall = tall.max(m.height);
        }
    }
    let x = cx - width / 2.0;
    for p in pieces {
        if p[0] {
            x += gap;
            x += draw_pad_glyph(x, y + tall / 2.0 - h / 2.0, p[1], h, alpha) + gap;
        } else {
            let words = p[1];
            draw_text_opts(x + 1.0, y + 1.0, words, size, #{ font: "strong", color: [0.0, 0.0, 0.0, 0.6 * alpha] });
            draw_text_opts(x, y, words, size, #{ font: "strong", color: [ink[0], ink[1], ink[2], alpha] });
            x += measure_text_ex(words, size, "strong", 0.0).width;
        }
    }
}

// How wide `draw_pad_glyph` draws `name`, `h` high.
fn draw_pad_glyph_width(name, h) {
    if pad_tint(name) != () { return h; }
    if ["Up", "Down", "Left", "Right"].contains(name) { return h; }
    (measure_text_ex(name, h * 0.6, "strong", 0.0).width + 10.0).max(h)
}

// What E acts on over the shoulder: a node's ring, a station's in gold, a
// ring at someone's feet, and by it what E (and Shift-E) will do. The hall,
// its board, the office and the stalls show as hovered instead, with their
// own prompts, above.
fn draw_focus(f, now) {
    if f.kind == "hall" || f.kind == "board" || f.kind == "office" || f.kind == "stall" { return; }
    let y = cap_mark_y();
    let c = gold();
    let r = f.r * (1.0 + 0.04 * (now * 2.4).sin());
    ring(f.x, f.z, y, r, c, 0.85, 2.0, 48);
    ring(f.x, f.z, y, r * 1.3, c, 0.25, 1.0, 56);
    if f.kind == "station" {
        let spin = now * 0.4;
        for i in 0..4 {
            let a = spin + i.to_float() * 0.5 * PI();
            arc(f.x, f.z, y, r + 0.22, a - 0.16, a + 0.16, c, 0.6, 1.5);
        }
    }
    draw_focus_prompt(f);
}

// The aim dot (T1.6): a gold dot on a dark edge at the screen's centre,
// where the captured cursor reads. `k` runs from 0, faint, with nothing in
// focus, to 1, full, with something (on_draw_ui eases it), so you can see
// E has a target before you read the prompt.
fn draw_aim(k) {
    let x = screen_width() / 2.0;
    let y = screen_height() / 2.0;
    let c = gold();
    let a = 0.42 + 0.58 * k;
    draw_circle(x, y, 4.2 + 0.6 * k, 0.06, 0.05, 0.04, 0.55 * a);
    draw_circle(x, y, 2.6 + 0.6 * k, c[0], c[1], c[2], a);
    // A faint ring opens round it with a focus, as rings do round the
    // focus itself
    if k > 0.02 {
        let r = 7.5 + 1.5 * k;
        for i in 0..24 {
            let t0 = i.to_float() / 24.0 * 2.0 * PI();
            let t1 = (i + 1).to_float() / 24.0 * 2.0 * PI();
            draw_line(x + r * t0.cos(), y + r * t0.sin(), x + r * t1.cos(), y + r * t1.sin(),
                      c[0], c[1], c[2], 0.35 * k, 1.2);
        }
    }
}

// "[E] Gather wild grass", and "[Shift E] Trade with Ada" under it: over a
// node or a station, under someone's feet, clear of the name over them.
// With a pad in your hands, (A) and (Y) instead (T2.2).
fn draw_focus_prompt(f) {
    let ground = cap_ground_at(f.x, f.z);
    let at = world_to_screen(f.x, ground + f.top, f.z);
    if !at.visible { return; }
    // With a pad in your hands, A acts and Y trades (T2.2)
    let pad = pad_held();
    let lines = [[if pad { "A" } else { "E" }, f.label]];
    if f.shift_label != () { lines.push([if pad { "Y" } else { "Shift E" }, f.shift_label]); }
    let size = 15.0;
    let gap = 7.0;
    let row = 24.0;
    let below = f.kind == "player";
    let height = row * lines.len().to_float();
    let top = if below { at.y + 14.0 } else { at.y - 12.0 - height };
    // Over the shoulder what's ahead stands just over your head on the
    // screen: lift the prompt clear of your own name rather than across it
    for t in cap_name_tags() {
        if !t.me || !t.named { continue; }
        let clear = t.y + t.size + 4.0;
        if top < clear && top + height > t.y - 4.0 && (t.x - at.x).abs() < 160.0 {
            top = t.y - 6.0 - height;
        }
    }
    for k in 0..lines.len() {
        let key = lines[k][0];
        let words = lines[k][1];
        let km = measure_text_ex(key, 12.0, "strong", 0.0);
        let wm = measure_text_ex(words, size, "strong", 0.0);
        let kw = if pad { 19.0 } else { km.width + 12.0 };
        let w = kw + gap + wm.width;
        let x = at.x - w / 2.0;
        let y = top + row * k.to_float();
        let a = if k == 0 { 1.0 } else { 0.8 };
        if pad {
            draw_pad_glyph(x, y, key, 19.0, a);
        } else {
            draw_rect_ex(x + 1.0, y + 2.0, kw, 19.0, 0.0, 0.0, 0.0, 0.35 * a, 4.0, 0);
            draw_rect_ex(x, y, kw, 19.0, 0.10, 0.09, 0.07, 0.82 * a, 4.0, 0);
            draw_rect_outline_ex(x, y, kw, 19.0, 0.79, 0.64, 0.15, 0.95 * a, 1.0, 4.0, 0);
            draw_text_opts(x + 6.0, y + (19.0 - km.height) / 2.0, key, 12.0, #{
                font: "strong", color: [0.96, 0.90, 0.72, a],
            });
        }
        let tx = x + kw + gap;
        let ty = y + (19.0 - wm.height) / 2.0;
        draw_text_opts(tx + 1.0, ty + 1.0, words, size, #{ font: "strong", color: [0.0, 0.0, 0.0, 0.6 * a] });
        draw_text_opts(tx, ty, words, size, #{ font: "strong", color: [0.98, 0.95, 0.88, 0.97 * a] });
    }
}

// The hall's marks: its footprint traced while you hover it or stand in
// range, the prompt over it, and the motes while goods wait.
fn draw_hall(h, over, now) {
    let c = trade_blue();
    if over || h.near {
        let a = if over { 0.8 } else { 0.42 + 0.12 * (now * 2.0).sin() };
        let y = cap_mark_y();
        for i in 0..4 {
            let p = h.corners[i];
            let q = h.corners[(i + 1) % 4];
            ground_line(p[0], p[1], q[0], q[1], y, c, a, 2.0);
        }
    }
    if h.goods { draw_motes(h, now); }
    // Over the shoulder only the focus speaks: "press E" while you're
    // looking elsewhere would be a lie
    let near = h.near && !is_cursor_captured();
    if (over || near) && !h.open { draw_hall_prompt(h, over); }
}

// "Exchange hall", and under it what a click will do, over the clock.
fn draw_hall_prompt(h, over) {
    let at = world_to_screen(h.door_x, h.y + 2.2, h.door_z);
    if !at.visible { return; }
    let title = "Exchange hall";
    let line = if !h.near { "Click to walk there" }
               else if h.goods { "Goods wait in your box · click to collect" }
               else if pad_held() { "Click to trade · {Right}" }
               else { "Click to trade · M" };
    let size = 18.0;
    let m = measure_text_ex(title, size, "display", 0.0);
    // Close up the clock is near the top of the screen: keep the prompt
    // under the header.
    let top = (at.y - 44.0).max(64.0);
    draw_text_opts(at.x - m.width / 2.0 + 1.0, top + 1.0, title, size, #{
        font: "display", color: [0.0, 0.0, 0.0, 0.6],
    });
    draw_text_opts(at.x - m.width / 2.0, top, title, size, #{
        font: "display", color: [0.86, 0.92, 1.0, 0.97],
    });
    let ly = top + m.height + 5.0;
    let ink = if h.goods && h.near { [0.95, 0.83, 0.46] } else { [0.92, 0.92, 0.92] };
    draw_act_line(at.x, ly, act_words(line), 13.0, ink, if over || h.near { 0.9 } else { 0.7 });
}

// ── The road in (#442) ───────────────────────────────────────────────────

// How often a ripple goes out from the lead's ring, and a mote rises from
// it (s), how long a mote takes, and how high it goes (m).
fn lead_ripple() { 2.8 }
fn lead_mote_every() { 0.9 }
fn lead_mote_life() { 2.6 }
fn lead_mote_rise() { 2.4 }

// Where the road in's card leads: a gold ring breathing slowly on the
// ground, a dashed one turning outside it, a ripple going out now and then,
// a lot's edge traced faintly, a glint hanging over it and a few motes
// rising from the ring. Slow on purpose: it's a place to go, not an alarm.
fn draw_lead(l, now) {
    let y = cap_mark_y();
    let c = gold();
    let r = l.r * (1.0 + 0.05 * (now * 1.2).sin());
    ring(l.x, l.z, y, r, c, 0.75, 2.0, 48);
    dashed_ring(l.x, l.z, y, r + 0.35, c, 0.3, 1.5, 24, now * 0.12);
    let k = (now % lead_ripple()) / lead_ripple();
    let e = 1.0 - (1.0 - k) * (1.0 - k);
    ring(l.x, l.z, y, r * (1.0 + 0.8 * e), c, 0.4 * (1.0 - k), 1.0, 48);
    let n = l.outline.len();
    if n > 1 {
        let glow = 0.5 + 0.12 * (now * 1.2).sin();
        for i in 0..n {
            let a = l.outline[i];
            let b = l.outline[(i + 1) % n];
            ground_line(a[0], a[1], b[0], b[1], y, c, glow, 2.0);
        }
    }
    let ground = cap_ground_at(l.x, l.z);
    let p = world_to_screen(l.x, ground + 2.2 + 0.12 * (now * 1.4).sin(), l.z);
    if p.visible {
        let gr = 6.0 * (0.55 + 0.45 * (now * 1.8).sin()) + 3.0;
        draw_circle(p.x, p.y, gr * 1.9, c[0], c[1], c[2], 0.12);
        mote_glint(p.x, p.y, gr, now * 0.4, 0.9);
    }
    let count = (lead_mote_life() / lead_mote_every()).ceiling().to_int();
    let first = (now / lead_mote_every()).floor().to_int();
    for j in 0..count {
        let i = first - j;
        let age = now - i.to_float() * lead_mote_every();
        let k = age / lead_mote_life();
        if k < 0.0 || k >= 1.0 { continue; }
        // Each from its own place round the ring
        let a = (i * 61 % 100).to_float() / 100.0 * 2.0 * PI();
        let up = lead_mote_rise() * (1.0 - (1.0 - k) * (1.0 - k));
        let q = world_to_screen(l.x + r * a.cos(), ground + 0.2 + up, l.z + r * a.sin());
        if !q.visible { continue; }
        let fade = (k / 0.2).min(1.0) * (1.0 - k);
        mote_glint(q.x, q.y, 3.0 + 2.0 * (1.0 - k), age * 1.3 + a, 0.8 * fade);
    }
}

// ── The bazaar (E3.11) ───────────────────────────────────────────────────

// The lanterns' amber, #e8a24a.
fn amber() { [0.910, 0.635, 0.290] }

// Every kept stall's name on its sign, near enough to read; and for the
// stall under the cursor, the one you stand at and the one you walk to,
// its footprint traced and a prompt over it. A clean capture (--clean)
// keeps only the signs: wherever the cursor rests, it isn't anyone's (#345).
fn draw_bazaar(b, over, now) {
    for s in b.stalls {
        if s.name != () { draw_sign(s); }
    }
    if cap_clean() { return; }
    for s in b.stalls {
        let hovered = over != () && over == s.id;
        let walking = b.walking != () && b.walking == s.id;
        if hovered || s.near || walking { draw_stall(s, hovered, walking, now); }
    }
}

// The shop's name painted on both faces of its sign, cream on the dark
// board with a sign-writer's shade, as wide as the board lets it run. It's
// in the world, so whatever stands in front of it hides it. Too far off to
// read, it fades out.
fn draw_sign(s) {
    let at = world_to_screen(s.x, s.y, s.z);
    if !at.visible { return; }
    let edge = world_to_screen(s.x, s.y + 0.3, s.z);
    if !edge.visible { return; }
    // How many points the board's 0.3 m height spans
    let px = (edge.y - at.y).abs();
    if px < 9.0 { return; }
    let fade = ((px - 9.0) / 6.0).min(1.0);
    let m = measure_text_ex(s.name, 20.0, "display", 0.0);
    let size = (s.sign_w * 20.0 / m.width).min(0.13);
    let d = s.sign_d + 0.004;
    for side in [1.0, -1.0] {
        let nx = s.fx * side;
        let nz = s.fz * side;
        draw_world_text(s.x + nx * d, s.y, s.z + nz * d, s.name, size, #{
            font: "display", color: [0.98, 0.93, 0.80, 0.97 * fade], facing: [nx, 0.0, nz],
            shadow: [size * 0.05, size * 0.05, 0.0, 0.0, 0.0, 0.6 * fade],
        });
    }
}

// A stall's footprint traced in amber, and over it its name (or "Stall n,
// to let") and what a click does.
fn draw_stall(s, over, walking, now) {
    let c = amber();
    let a = if over { 0.85 } else if walking { 0.35 + 0.3 * (now * 3.0).sin().abs() } else { 0.42 + 0.12 * (now * 2.0).sin() };
    let y = cap_mark_y();
    for i in 0..4 {
        let p = s.corners[i];
        let q = s.corners[(i + 1) % 4];
        ground_line(p[0], p[1], q[0], q[1], y, c, a, 2.0);
    }
    // Only the stall under the cursor speaks: from the lane's crown four
    // are in range, and four prompts would bury each other.
    if s.open || !over { return; }
    let at = world_to_screen(s.x, s.y + 1.4, s.z);
    if !at.visible { return; }
    let title = if s.name != () { s.name } else { "Stall " + s.number + ", to let" };
    let line = if s.mine { "Your shop · click to tend it" }
               else if s.name == () { if s.near { "20 Strands a day · click to rent" } else { "20 Strands a day · click to walk there" } }
               else if s.near { "Kept by " + s.keeper + " · click to browse" }
               else { "Kept by " + s.keeper + " · click to walk there" };
    let size = 17.0;
    let m = measure_text_ex(title, size, "display", 0.0);
    let top = (at.y - 44.0).max(64.0);
    draw_text_opts(at.x - m.width / 2.0 + 1.0, top + 1.0, title, size, #{ font: "display", color: [0.0, 0.0, 0.0, 0.6] });
    draw_text_opts(at.x - m.width / 2.0, top, title, size, #{ font: "display", color: [1.0, 0.92, 0.78, 0.97] });
    let ly = top + m.height + 5.0;
    draw_act_line(at.x, ly, act_words(line), 13.0, [0.95, 0.83, 0.62], 0.92);
}

// ── The Gazette's notice board (E3.9) ────────────────────────────────────

// Its name and what a click does, over it while you hover it; and while an
// edition waits unread, or a commission is pinned there that your pack
// could fill (S2.3), a gold glint breathing over its lamp.
fn draw_board(h, over, now) {
    let fill = h.fill > 0;
    if (h.news || fill) && !h.reading { draw_news_glint(h, now); }
    if !over || h.reading { return; }
    let at = world_to_screen(h.board_x, h.board_y + 0.6, h.board_z);
    if !at.visible { return; }
    let title = "The Notice Board";
    let line = if h.news {
        "A new edition · click to read"
    } else if fill {
        `${h.fill} wanted that you could fill`
    } else {
        "Wanted, and the Gazette · click to read"
    };
    let size = 16.0;
    let m = measure_text_ex(title, size, "display", 0.0);
    let top = (at.y - 40.0).max(64.0);
    draw_text_opts(at.x - m.width / 2.0 + 1.0, top + 1.0, title, size, #{
        font: "display", color: [0.0, 0.0, 0.0, 0.6],
    });
    draw_text_opts(at.x - m.width / 2.0, top, title, size, #{
        font: "display", color: [0.96, 0.92, 0.82, 0.97],
    });
    let ly = top + m.height + 5.0;
    let ink = if h.news || fill { [0.95, 0.83, 0.46] } else { [0.92, 0.92, 0.92] };
    draw_act_line(at.x, ly, act_words(line), 13.0, ink, 0.92);
}

// ── The Deeds Office (#448) ──────────────────────────────────────────────

// Its name and what a click does, over it while you hover it; and while
// your deed waits, earned and unspent, a gold glint over its awning, slower
// than the news's: a thing kept for you, not news.
fn draw_office(h, over, now) {
    if h.deed_waits && !h.office_open { draw_deed_glint(h, now); }
    if !over || h.office_open { return; }
    let at = world_to_screen(h.office_x, h.office_y + 0.6, h.office_z);
    if !at.visible { return; }
    let title = "The Deeds Office";
    let line = if h.deed_waits { "Your deed waits here · click to look in" } else { "Click to compare the zones" };
    let size = 16.0;
    let m = measure_text_ex(title, size, "display", 0.0);
    let top = (at.y - 40.0).max(64.0);
    draw_text_opts(at.x - m.width / 2.0 + 1.0, top + 1.0, title, size, #{
        font: "display", color: [0.0, 0.0, 0.0, 0.6],
    });
    draw_text_opts(at.x - m.width / 2.0, top, title, size, #{
        font: "display", color: [0.98, 0.9, 0.84, 0.97],
    });
    let ly = top + m.height + 5.0;
    let ink = if h.deed_waits { [0.95, 0.83, 0.46] } else { [0.92, 0.92, 0.92] };
    draw_act_line(at.x, ly, act_words(line), 13.0, ink, 0.92);
}

// The glint over the office's awning: the motes' four-pointed star, turning
// slowly, over a soft gold halo.
fn draw_deed_glint(h, now) {
    let p = world_to_screen(h.office_x, h.office_y + 0.3, h.office_z);
    if !p.visible { return; }
    let swell = 0.55 + 0.45 * (now * 1.1).sin();
    let r = 6.0 * swell + 3.5;
    let c = gold();
    draw_circle(p.x, p.y, r * 1.8, c[0], c[1], c[2], 0.14 * swell + 0.04);
    mote_glint(p.x, p.y, r, now * 0.25, 0.9);
}

// A four-pointed glint over the board's lamp, swelling and fading slowly,
// as the hall's motes do: a few points wide however far away you stand.
fn draw_news_glint(h, now) {
    let p = world_to_screen(h.board_x, h.board_y + 0.35, h.board_z);
    if !p.visible { return; }
    let swell = 0.55 + 0.45 * (now * 1.8).sin();
    let r = 7.0 * swell + 3.0;
    let w = 1.6;
    let a = 0.85;
    draw_polygon([[p.x, p.y - r], [p.x + w, p.y], [p.x, p.y + r], [p.x - w, p.y]], 1.0, 0.92, 0.6, a, 2);
    draw_polygon([[p.x - r, p.y], [p.x, p.y - w], [p.x + r, p.y], [p.x, p.y + w]], 1.0, 0.92, 0.6, a, 2);
    draw_circle(p.x, p.y, 2.2, 1.0, 0.97, 0.85, a);
}

// Goods in your box: gold motes rising one after another from the hall's
// lantern, swaying as they climb, swelling then fading, with a faint
// thread of gold under them. Each is a glint on the screen, so it stays a
// few points wide however far away you are.
fn draw_motes(h, now) {
    let c = gold();
    let n = (mote_life() / mote_every()).ceiling().to_int();
    let first = (now / mote_every()).floor().to_int();
    for j in 0..n {
        let i = first - j;
        let age = now - i.to_float() * mote_every();
        let k = age / mote_life();
        if k < 0.0 || k >= 1.0 { continue; }
        // Each mote its own sway, from its number.
        let phase = (i * 37 % 100).to_float() / 100.0 * 2.0 * PI();
        let up = mote_rise() * (1.0 - (1.0 - k) * (1.0 - k));
        let sway = 0.45 * (phase + age * 1.7).sin() * k;
        let p = world_to_screen(h.x + sway, h.y + 0.4 + up, h.z + 0.3 * (phase + age).cos() * k);
        if !p.visible { continue; }
        let a = (k / 0.15).min(1.0) * (1.0 - k);
        let r = 4.0 + 5.0 * (1.0 - (2.0 * k - 1.0).abs());
        draw_circle(p.x, p.y, r * 1.9, c[0], c[1], c[2], 0.18 * a);
        mote_glint(p.x, p.y, r, age * 1.3 + phase, a);
    }
    // The thread: depth-tested, so the hall hides its foot as it should.
    let base = h.y + 0.6;
    let steps = 8;
    for s in 0..steps {
        let y0 = base + mote_rise() * s.to_float() / steps.to_float();
        let y1 = base + mote_rise() * (s + 1).to_float() / steps.to_float();
        let fade = 0.28 * (1.0 - s.to_float() / steps.to_float());
        draw_world_line(h.x, y0, h.z, h.x, y1, h.z, c[0], c[1], c[2], fade, 1.5);
    }
}

// A four-pointed glint, gold-white, `r` points to a tip, turned by `turn`.
fn mote_glint(x, y, r, turn, a) {
    let w = r * 0.24;
    for k in 0..2 {
        let t = turn + k.to_float() * 0.5 * PI();
        let ux = t.cos();
        let uy = t.sin();
        draw_polygon([
            [x + ux * r, y + uy * r], [x - uy * w, y + ux * w],
            [x - ux * r, y - uy * r], [x + uy * w, y - ux * w],
        ], 1.0, 0.92, 0.6, a, 2);
    }
    draw_circle(x, y, w * 1.3, 1.0, 0.98, 0.9, a);
}

// Someone shift would ask: a blue ring at their feet, closing in a little
// as it breathes, with a fainter one outside.
fn draw_trade_ring(p, now) {
    let y = feet_mark(p.x, p.z, p.feet_y);
    let c = trade_blue();
    let r = 0.85 + 0.05 * (now * 3.0).sin();
    ring(p.x, p.z, y, r, c, 0.85, 2.0, 40);
    ring(p.x, p.z, y, r * 1.45, c, 0.25, 1.0, 48);
}

// Under a player's feet at (x, z): a little over them, and never under
// the marks' layer, so a lot or a road doesn't hide the ring. As the
// marks' own height, less the land's rise, which mark_line adds back.
fn feet_mark(x, z, feet_y) { (feet_y - cap_rise(x, z) + 0.03).max(cap_mark_y()) }

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
    let feet = if you != () { you[1] } else { cap_ground_at(d.x, d.z) };
    let ease = 1.0 - (1.0 - k) * (1.0 - k);
    for end in [[d.x, d.z], [d.tx, d.tz]] {
        ring(end[0], end[1], feet_mark(end[0], end[1], feet), 0.5 + 1.8 * ease, g, 0.85 * (1.0 - k), 2.0, 44);
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
            mark_line(a[0] + (b[0] - a[0]) * t0, p.y, a[1] + (b[1] - a[1]) * t0,
                      a[0] + (b[0] - a[0]) * t1, p.y, a[1] + (b[1] - a[1]) * t1,
                      c, (base + 0.5 * lit).min(1.0), width + lit);
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
        mark_line(g[0] + r * 1.25 * a.cos(), y, g[1] + r * 1.25 * a.sin(),
                  g[0] + r * 1.7 * a.cos(), y, g[1] + r * 1.7 * a.sin(),
                  c, 0.7, 1.5);
    }
}

fn smooth(t) {
    let t = t.max(0.0).min(1.0);
    t * t * (3.0 - 2.0 * t)
}
