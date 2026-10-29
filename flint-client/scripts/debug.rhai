// The admin's world layers (D5.3, devtools plan §6). F6 steps through them,
// for an admin's token only, and Rust gathers what each shows into
// cap_debug() (debug_layers.rs); this only draws it, in the world, behind
// whatever stands in front of it:
//
//   interest   your cell of the server's interest grid and the eight round
//              it, the square it sends you players from: yours bright, the
//              others faint, each named at its middle.
//   nodes      every node near you on a stalk, with its kind and the ground
//              it stands on (height, slope, massif), and in the wilds what
//              its habitat in wilds.toml makes of that ground (#413): green
//              with a ring at its foot where it suits, rose where it
//              doesn't, and why. Unjudged ones are pale, spent ones grey.
//   walks      the walk graph round you, that every click and every citizen
//              walks on.
//   walkers    where the server is walking each player near you: a line to
//              the end of the leg, a ring there, and what they're doing.
//              Citizens in their lantern's pale blue, people in gold.
//
// The colours are their own, apart from the game's marks, so a layer never
// reads as something you can click.

fn cell_blue() { [0.36, 0.78, 0.95] }
fn node_green() { [0.45, 0.86, 0.48] }
fn wrong_rose() { [1.0, 0.45, 0.47] }
fn unjudged_pale() { [0.84, 0.86, 0.78] }
fn spent_grey() { [0.6, 0.6, 0.6] }
fn graph_violet() { [0.72, 0.52, 1.0] }
fn citizen_blue() { [0.80, 0.88, 1.0] }
fn person_gold() { [0.98, 0.78, 0.32] }

// Metres a line is cut into, so it lies on the land as it rises.
fn piece() { 2.0 }
// How high a node's stalk stands, in metres.
fn stalk() { 2.6 }

fn on_draw_ui() {
    if cap_at_login() { return; }
    let d = cap_debug();
    if d == () { return; }
    let y = cap_mark_y();
    for c in d.cells { draw_cell(c, y); }
    for e in d.edges { draw_edge(e, y); }
    for n in d.nodes { draw_node(n, y); }
    for w in d.walkers { draw_walker(w, y); }
}

// A line along the ground from (ax, az) to (bx, bz), in pieces that each
// take the land's rise at their ends.
fn ground_run(ax, az, bx, bz, y, c, alpha, width) {
    let dx = bx - ax;
    let dz = bz - az;
    let span = (dx * dx + dz * dz).sqrt();
    let n = (span / piece()).ceiling().to_int().max(1);
    let px = ax;
    let pz = az;
    for i in 1..=n {
        let t = i.to_float() / n.to_float();
        let qx = ax + dx * t;
        let qz = az + dz * t;
        draw_world_line(px, y + cap_rise(px, pz), pz, qx, y + cap_rise(qx, qz), qz,
                        c[0], c[1], c[2], alpha, width);
        px = qx;
        pz = qz;
    }
}

fn label(x, z, lift, text, size, c, alpha) {
    draw_world_text(x, cap_ground_at(x, z) + lift, z, text, size, #{
        font: "strong", color: [c[0], c[1], c[2], alpha],
        shadow: [1.0, 1.0, 0.0, 0.0, 0.0, 0.7 * alpha],
    });
}

fn draw_cell(c, y) {
    let col = cell_blue();
    let a = if c.mine { 0.85 } else { 0.35 };
    let w = if c.mine { 2.5 } else { 1.5 };
    ground_run(c.x0, c.z0, c.x1, c.z0, y, col, a, w);
    ground_run(c.x1, c.z0, c.x1, c.z1, y, col, a, w);
    ground_run(c.x1, c.z1, c.x0, c.z1, y, col, a, w);
    ground_run(c.x0, c.z1, c.x0, c.z0, y, col, a, w);
    let text = if c.mine { `${c.label}, yours` } else { c.label };
    label((c.x0 + c.x1) / 2.0, (c.z0 + c.z1) / 2.0, 1.0, text, if c.mine { 16.0 } else { 13.0 }, col, a);
}

fn draw_edge(e, y) {
    ground_run(e[0], e[1], e[2], e[3], y + 0.03, graph_violet(), 0.6, 1.5);
}

fn draw_node(n, y) {
    // A node on the wrong ground is rose even spent: that's what to find
    let c = if n.suits == false { wrong_rose() }
            else if !n.available { spent_grey() }
            else if n.suits == true { node_green() }
            else { unjudged_pale() };
    let g = cap_ground_at(n.x, n.z);
    draw_world_line(n.x, g, n.z, n.x, g + stalk(), n.z, c[0], c[1], c[2], 0.85, 2.0);
    if n.suits != () { ring(n.x, n.z, 0.7, y + 0.04, c, 0.8); }
    // One line, so the parts never overlap however far the camera stands
    let text = if n.ground == "" { n.kind } else { `${n.kind} · ${n.ground}` };
    if n.fit != "" { text += ` · ${n.fit}`; }
    label(n.x, n.z, stalk() + 0.3, text, 12.0, c, 0.95);
}

// A ring of radius r on the ground round (x, z).
fn ring(x, z, r, y, c, alpha) {
    let steps = 20;
    let px = x + r;
    let pz = z;
    for i in 1..=steps {
        let a = 2.0 * PI() * i.to_float() / steps.to_float();
        let qx = x + r * a.cos();
        let qz = z + r * a.sin();
        draw_world_line(px, y + cap_rise(px, pz), pz, qx, y + cap_rise(qx, qz), qz,
                        c[0], c[1], c[2], alpha, 2.0);
        px = qx;
        pz = qz;
    }
}

fn draw_walker(w, y) {
    let c = if w.citizen { citizen_blue() } else { person_gold() };
    if w.dest != () {
        let dx = w.dest[0];
        let dz = w.dest[1];
        ground_run(w.x, w.z, dx, dz, y + 0.05, c, 0.75, 2.0);
        ring(dx, dz, 0.9, y + 0.05, c, 0.9);
        label(dx, dz, 1.8, `${w.name}'s leg`, 12.0, c, 0.85);
    }
    if w.doing != "" {
        label(w.x, w.z, 3.2, w.doing, 12.0, c, 0.9);
    }
}
