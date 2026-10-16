// The camera rig (P3.9). The camera follows a point on the ground with
// damping. Zoom runs on one dial, from standing over someone's shoulder to
// the whole world seen from above, and the tilt rides on it:
//
//   - closest (11 m off): 15° down and under 3 m up, standing in the
//     street with the sky in the top third of the frame, and from the
//     outer zones the tower on the horizon;
//   - street (32 m): about 50° down, where the camera rests;
//   - map (640 m): straight down, north at the top, the light flattened
//     into the 2D client's palette (atmosphere.rs keys that on height).
//
// On the way to the map the view drifts to the Capitol and turns north up,
// so the furthest zoom is always the same map.
//
// Controls (config/input.toml): WASD or the arrows pan, R and F or the
// wheel zoom, Q and E or a right-drag orbit, H comes home. The wheel
// arrives through cap_take_wheel(), because Flint doesn't feed it to
// actions yet (issue #67). The keys stand in for Home, PageUp and +/-,
// which Flint can't bind yet (issue #68).
//
// Signed in, the point is your avatar (cap_follow, P4.2), and WASD pans
// the view away from it: the pan stays put relative to you as you walk,
// and H brings it home to you. Offline there's no avatar, so the point is
// a free one WASD moves, drawn as a gold mark.
//
// While the login form is up (cap_at_login, P4.1) the rig takes no input.
// The camera circles the tower slowly from beyond the canals, with the
// tower held right of centre so the form on the left doesn't cover it.
// Once the player is in, that circle is let go like any fixed shot, and
// the camera comes down out of it onto your avatar. It keeps circling
// until the server has said where you are, for at most `landing_wait`.
//
// Walking into a new zone (P8.2), the camera eases out a little, up and
// back along its line to you, so the zone you've come into opens out round
// you while its banner names it (hud.rhai), and then eases back in. It
// doesn't take the zoom: it's on top of the rig, so a wheel turned meanwhile
// still counts. It fades away toward the map, where there's nothing to open.
//
// `--view` (cap_view) frames a capture. street, district and map are rig
// framings. The rest are fixed shots: bridge (the quay by the inner canal's
// south crossing), suburb (low over the southern suburbs' curving streets),
// urban (close over the lots by the south arterial), capitol (up at the
// tower from beyond the moat), crown (level with its lantern), and
// from-urban, from-suburban, from-rural and from-wilderness (the tower on
// the horizon from each outer zone). A shot holds until the first camera
// input, then eases into the rig.
//
// The player's settings (P8.4) scale how fast it turns and zooms
// (cap_setting("camera_sensitivity")) and can turn the orbit the other way
// (cap_setting("invert_orbit")). The rates below are the rig as made.
//
// rig_pose and the functions under it are pure, and a Rust test
// (camera_rig.rs) drives them across every zoom, yaw and position.

// ── Tuning ───────────────────────────────────────────────────────────────

// Camera distance from the point it looks at (m) at zoom 0 and zoom 1.
// Zoom is linear in the log of distance, so each wheel notch feels alike.
fn near_distance() { 11.0 }
fn far_distance() { 640.0 }
// Where the camera rests: 32 m off, 50° down, about 25 m up.
fn street_distance() { 32.0 }
fn street_zoom() {
    (street_distance() / near_distance()).ln() / (far_distance() / near_distance()).ln()
}
fn district_zoom() { 0.75 }
// Tilt (degrees below level) at zoom 0, at street and at the map. Just
// short of 90 at the map, so the look-at keeps a heading.
fn closest_pitch() { 15.0 }
fn street_pitch() { 50.0 }
fn map_pitch() { 89.9 }
// The view drifts from the point to the Capitol over this zoom range, and
// turns north up over the second one.
fn centre_from() { 0.62 }
fn north_from() { 0.80 }
// Keep the free point on the world (its ground fades out by r400).
fn world_radius() { 420.0 }
// How fast things settle: 1/e in 1/k seconds.
fn follow_k() { 6.0 }
// The nudge when a canal stops you (P4.3): how long it rings, and how far
// it swings as a share of the camera's distance from what it looks at.
fn bump_life() { 0.7 }
fn bump_size() { 0.025 }
// The ease on entering a zone: how much further out the camera stands at
// its widest, as a share of its distance, how long it takes to get there,
// when it starts back and how long that takes (s). Timed to the banner.
fn arrive_size() { 0.18 }
fn arrive_out() { 1.2 }
fn arrive_back() { 1.7 }
fn arrive_return() { 2.0 }
fn zoom_k() { 8.0 }
fn yaw_k() { 10.0 }
// Input rates.
fn wheel_step() { 0.045 }   // zoom per wheel notch
fn key_zoom_rate() { 0.45 } // zoom per second on R/F
fn orbit_rate() { 1.6 }     // radians per second on Q/E
fn drag_rate() { 0.006 }    // radians per point of right-drag
// How long a fixed shot takes to ease into the rig, in seconds, and how
// long the camera takes to come down from the login circle onto you.
fn release_time() { 1.0 }
fn landing_time() { 2.2 }
// How long the circle waits past the form for where you are (s).
fn landing_wait() { 1.5 }
// The circle behind the login form: radius and height (m), where it
// starts (radians round the tower, 0 south; -0.6 is south-west, facing the
// sun's side), how fast it turns (one lap in about three minutes), what it
// looks at, and how far that's shifted left so the tower sits right of
// the form.
fn login_radius() { 118.0 }
fn login_height() { 34.0 }
fn login_start() { -0.6 }
fn login_rate() { 0.035 }
fn login_look_y() { 34.0 }
fn login_shift() { 40.0 }

// The Capitol's keep, crown and needle as a column the camera stays out
// of (world/tower.rs: its corner turrets reach 9.6 m out, the needle's
// tip under 84 m). Over the whole bailey the camera keeps above the curtain
// wall, its towers (which stand inside it, from r13.4) and its gatehouses,
// and it eases down to the street past the moat. camera_rig.rs holds these
// to the tower as built.
fn keep_radius() { 11.0 }
fn keep_top() { 86.0 }
fn wall_outer() { 22.5 }
fn wall_clear() { 10.0 } // above the land: over the gatehouse (8.8 m) by the near plane
fn ground_clear() { 1.5 }

// ── State ────────────────────────────────────────────────────────────────

let rig = #{
    // The point followed, and where the camera looks, damped toward it.
    target_x: 0.0, target_z: 48.0,
    focus_x: 0.0, focus_z: 48.0,
    zoom: street_zoom(), zoom_to: street_zoom(),
    // Radians round the point. 0 puts the camera south, looking north.
    yaw: 0.0, yaw_to: 0.0,
    // Cursor x last frame while the right button is held.
    drag_x: (),
    // A --view shot: #{ pos, at } held until camera input, then `release`
    // runs 0 -> 1 as it eases into the rig.
    shot: (), release: 1.0, release_len: 1.0,
    started: false,
    // The login circle held last frame, and how long it has waited past
    // the form for where you are.
    at_login: false, waited: 0.0,
    // How far the view is panned from your avatar (m). Unused offline.
    pan_x: 0.0, pan_z: 0.0,
    // A canal stopped you (P4.3): #{ born, dx, dz }, the way you were
    // going, for a nudge toward the water and back. () when still.
    bump: (),
    // The zone last seen in the banner, and when you walked into it (P8.2).
    zone: (), arrived: (),
};

// ── Per frame ────────────────────────────────────────────────────────────

fn on_update() {
    // A hitch mustn't fling the camera across the city.
    let dt = delta_time().min(0.1);
    if !rig.started {
        rig.started = true;
        rig.start_view(cap_view());
    }

    let moved = false;
    let you = cap_follow();
    let waiting = !cap_at_login() && rig.at_login && you == () && rig.waited < landing_wait();
    if cap_at_login() || waiting {
        rig.waited = if waiting { rig.waited + dt } else { 0.0 };
        rig.shot = login_shot(total_time());
        rig.release = 1.0;
        rig.at_login = true;
        // The wheel turned over the form isn't for later.
        cap_take_wheel();
    } else if rig.at_login {
        rig.at_login = false;
        rig.waited = 0.0;
        rig.take_over_from_shot();
        if you != () {
            // Down onto you, not onto wherever the circle was.
            rig.target_x = you[0];
            rig.target_z = you[2];
            rig.focus_x = you[0];
            rig.focus_z = you[2];
            rig.pan_x = 0.0;
            rig.pan_z = 0.0;
            rig.release_len = landing_time();
        }
    } else {
        moved = rig.steer(dt, you);
    }
    if you != () && !rig.at_login {
        rig.target_x = you[0] + rig.pan_x;
        rig.target_z = you[2] + rig.pan_z;
    }
    if moved && rig.shot != () && rig.release >= 1.0 {
        rig.take_over_from_shot();
    }

    // Damping: exponential, so it's the same at any frame rate.
    let f = 1.0 - (-follow_k() * dt).exp();
    rig.focus_x += (rig.target_x - rig.focus_x) * f;
    rig.focus_z += (rig.target_z - rig.focus_z) * f;
    rig.zoom += (rig.zoom_to - rig.zoom) * (1.0 - (-zoom_k() * dt).exp());
    rig.yaw += (rig.yaw_to - rig.yaw) * (1.0 - (-yaw_k() * dt).exp());

    let pose = rig_pose(rig.focus_x, rig.focus_z, rig.zoom, rig.yaw);
    if rig.shot != () {
        // Held, or easing out of it.
        let s = rig.shot;
        if rig.release < 1.0 {
            rig.release = (rig.release + dt / rig.release_len).min(1.0);
            let t = smooth(rig.release);
            pose = #{
                px: lerp(s.pos[0], pose.px, t), py: lerp(s.pos[1], pose.py, t),
                pz: lerp(s.pos[2], pose.pz, t),
                tx: lerp(s.at[0], pose.tx, t), ty: lerp(s.at[1], pose.ty, t),
                tz: lerp(s.at[2], pose.tz, t),
            };
            if rig.release >= 1.0 { rig.shot = (); }
        } else {
            pose = #{ px: s.pos[0], py: s.pos[1], pz: s.pos[2],
                      tx: s.at[0], ty: s.at[1], tz: s.at[2] };
        }
    }
    rig.watch_zone();
    if rig.shot == () { pose = rig.eased_in(pose); }
    pose = rig.nudged(pose);
    set_camera_position(pose.px, pose.py, pose.pz);
    set_camera_target(pose.tx, pose.ty, pose.tz);
}

// The water stopped your walk (input.rhai draws the splash): the view
// lurches a little on toward it and settles back.
fn on_event(name, data) {
    if name == "movement_blocked" {
        rig.bump = #{ born: total_time(), dx: data.dir_x, dz: data.dir_z };
    }
}

// The pose shifted by the bump: a damped swing along the way you were
// going, with a small dip, sized to how far the camera is from what it
// looks at, so it reads the same at street and district zoom.
fn nudged(pose) {
    if this.bump == () { return pose; }
    let age = total_time() - this.bump.born;
    if age > bump_life() { this.bump = (); return pose; }
    let dist = ((pose.px - pose.tx) * (pose.px - pose.tx)
              + (pose.py - pose.ty) * (pose.py - pose.ty)
              + (pose.pz - pose.tz) * (pose.pz - pose.tz)).sqrt();
    let a = (dist * bump_size()).min(2.5) * (-age * 7.0).exp() * (age * 15.0).sin();
    let dx = this.bump.dx * a;
    let dz = this.bump.dz * a;
    let dy = -0.35 * a.abs();
    #{ px: pose.px + dx, py: pose.py + dy, pz: pose.pz + dz,
       tx: pose.tx + dx, ty: pose.ty + dy, tz: pose.tz + dz }
}

// A new zone in the banner: start the ease. The zone you come into the
// world in only sets where you are; the landing is its ease.
fn watch_zone() {
    let b = cap_zone_banner();
    if b == () { this.zone = (); return; }
    if b.age < 0.0 || b.name == this.zone { return; }
    if this.zone != () && !b.first { this.arrived = total_time() - b.age; }
    this.zone = b.name;
}

// The pose eased out for a zone just entered: the camera moved along its
// line to what it looks at, out and then back.
fn eased_in(pose) {
    if this.arrived == () { return pose; }
    let age = total_time() - this.arrived;
    if age > arrive_back() + arrive_return() { this.arrived = (); return pose; }
    let e = smooth(age / arrive_out()) * (1.0 - smooth((age - arrive_back()) / arrive_return()));
    let k = arrive_size() * e * (1.0 - smooth((this.zoom - centre_from()) / 0.15));
    #{ px: pose.px + (pose.px - pose.tx) * k, py: pose.py + (pose.py - pose.ty) * k,
       pz: pose.pz + (pose.pz - pose.tz) * k, tx: pose.tx, ty: pose.ty, tz: pose.tz }
}

// Read this frame's camera input into the rig (`this`). Returns whether
// there was any, which is what lets go of a fixed shot. `you` is
// cap_follow(): with an avatar, WASD moves the pan from it.
fn steer(dt, you) {
    let any = false;

    // Move the point, relative to the way the camera faces. Faster the
    // further out the camera is, so a screen's width takes the same time.
    let fwd = action_axis("move_forward", "move_backward");
    let side = action_axis("move_right", "move_left");
    if fwd != 0.0 || side != 0.0 {
        any = true;
        let speed = (0.6 * rig_distance(this.zoom)).max(8.0) * dt;
        let len = (fwd * fwd + side * side).sqrt();
        let sy = this.yaw.sin();
        let cy = this.yaw.cos();
        // Forward is from the camera to the point: (-sin, -cos) in x, z.
        // Right is (cos, -sin).
        this.target_x += (-sy * fwd + cy * side) / len * speed;
        this.target_z += (-cy * fwd - sy * side) / len * speed;
        let r = (this.target_x * this.target_x + this.target_z * this.target_z).sqrt();
        if r > world_radius() {
            this.target_x *= world_radius() / r;
            this.target_z *= world_radius() / r;
        }
        if you != () {
            this.pan_x = this.target_x - you[0];
            this.pan_z = this.target_z - you[2];
        }
    }

    let feel = sensitivity();
    let way = if cap_setting("invert_orbit") == true { -1.0 } else { 1.0 };

    // Zoom: the wheel (up is in) and R/F.
    let notches = cap_take_wheel();
    let keys = action_axis("zoom_out", "zoom_in");
    if notches != 0.0 || keys != 0.0 {
        any = true;
        this.zoom_to = (this.zoom_to - (notches * wheel_step() - keys * key_zoom_rate() * dt) * feel)
            .max(0.0).min(1.0);
    }

    // Orbit: Q turns the view left, E right, and so does dragging right.
    let turn = action_axis("orbit_left", "orbit_right");
    if turn != 0.0 {
        any = true;
        this.yaw_to += turn * orbit_rate() * dt * feel * way;
    }
    if is_mouse_pressed(1) {
        let x = mouse_x();
        if this.drag_x != () && x != this.drag_x {
            any = true;
            this.yaw_to -= (x - this.drag_x) * drag_rate() * feel * way;
        }
        this.drag_x = x;
    } else {
        this.drag_x = ();
    }

    // Home: back to the start, at street zoom, north up. The yaw unwinds
    // by the short way, never a full turn.
    if is_action_just_pressed("home") {
        any = true;
        let home = follow_home(you);
        this.target_x = home[0];
        this.target_z = home[1];
        this.pan_x = 0.0;
        this.pan_z = 0.0;
        this.zoom_to = street_zoom();
        this.yaw_to = this.yaw - wrap_angle(this.yaw);
    }
    any
}

// The settings' camera sensitivity, or 1 if there's none to read.
fn sensitivity() {
    let s = cap_setting("camera_sensitivity");
    if type_of(s) == "f64" { s } else { 1.0 }
}

// Where H brings the point back to: you, or offline where the rig starts.
fn follow_home(you) {
    if you != () { [you[0], you[2]] } else { [0.0, 48.0] }
}

// +1 while `plus` is held, -1 while `minus` is, 0 for both or neither.
fn action_axis(plus, minus) {
    let v = 0.0;
    if is_action_pressed(plus) { v += 1.0; }
    if is_action_pressed(minus) { v -= 1.0; }
    v
}

// Frame `view` (from --view) at start: a rig framing or a fixed shot.
fn start_view(view) {
    let at = (); // the rig's focus as [x, z]
    if view == "map" {
        this.zoom = 1.0;
    } else if view == "district" {
        this.zoom = district_zoom();
        at = [0.0, 30.0];
    } else if view == "street" || view == "" {
        at = [0.0, 48.0];
    } else {
        this.shot = view_shot(view);
        this.release = 1.0;
    }
    if at != () {
        this.target_x = at[0];
        this.target_z = at[1];
    }
    this.zoom_to = this.zoom;
    this.focus_x = this.target_x;
    this.focus_z = this.target_z;
}

// The shot hands over to the rig, at street zoom, keeping the camera
// where it stood: the point followed is just ahead of it, the way it
// faced. Then the camera eases across from the shot.
fn take_over_from_shot() {
    let s = this.shot;
    let dx = s.at[0] - s.pos[0];
    let dz = s.at[2] - s.pos[2];
    let len = (dx * dx + dz * dz).sqrt();
    if len < 0.001 {
        // Straight down: north up.
        dx = 0.0;
        dz = -1.0;
        len = 1.0;
    }
    let ahead = rig_distance(street_zoom()) * street_pitch().to_radians().cos();
    this.target_x = s.pos[0] + dx / len * ahead;
    this.target_z = s.pos[2] + dz / len * ahead;
    this.focus_x = this.target_x;
    this.focus_z = this.target_z;
    this.yaw = atan(-dx, -dz);
    this.yaw_to = this.yaw;
    this.zoom = street_zoom();
    this.zoom_to = street_zoom();
    this.release = 0.0;
    this.release_len = release_time();
}

// The camera behind the login form at game time t: on its circle round the
// tower, looking at the tower with the aim shifted left. Right of the
// camera, looking in, is (cos a, -sin a).
fn login_shot(t) {
    let a = login_start() + t * login_rate();
    let s = a.sin();
    let c = a.cos();
    #{
        pos: [login_radius() * s, login_height(), login_radius() * c],
        at: [-c * login_shift(), login_look_y(), s * login_shift()],
    }
}

fn view_shot(view) {
    if view == "bridge" {
        // On the quay by the inner canal's south crossing (server angle 90),
        // looking along the water to the gap.
        #{ pos: [7.0, 3.4, 65.5], at: [-3.0, 0.0, 60.0] }
    } else if view == "suburb" {
        // Low over the southern suburbs, where the streets curve and branch
        // into cul-de-sacs.
        #{ pos: [-35.0, 45.0, 245.0], at: [-35.0, 0.0, 200.0] }
    } else if view == "urban" {
        // Close over the urban lots west of the south arterial, looking
        // north-east across the arterial at their curbs.
        #{ pos: [-14.0, 9.0, 142.0], at: [4.0, 0.0, 124.0] }
    } else if view == "capitol" {
        // On the trade district's quay south of the moat, looking up past
        // the gatehouse at the keep and its crown.
        #{ pos: [9.0, 3.0, 52.0], at: [0.0, 32.0, 0.0] }
    } else if view == "crown" {
        // Level with the crown from the south-west, for its light.
        #{ pos: [-14.0, 64.0, 22.0], at: [0.0, 66.0, 0.0] }
    } else if view == "from-urban" {
        // The tower on the horizon from a street-height camera in each
        // outer zone, each from its own side of the city.
        #{ pos: [-92.0, 10.0, 92.0], at: [0.0, 30.0, 0.0] }
    } else if view == "from-suburban" {
        #{ pos: [141.0, 16.0, 141.0], at: [0.0, 30.0, 0.0] }
    } else if view == "from-rural" {
        #{ pos: [145.0, 22.0, -251.0], at: [0.0, 30.0, 0.0] }
    } else {
        #{ pos: [-348.0, 26.0, -127.0], at: [0.0, 30.0, 0.0] }
    }
}

// Offline, the free point, marked in gold: gold means you. A dot and a
// ring on the screen, a constant size at every zoom. Signed in, your
// avatar carries the gold instead (entities.rs).
fn on_draw_ui() {
    if cap_follow() != () || cap_at_login() { return; }
    if rig.shot != () && rig.release >= 1.0 { return; }
    let p = world_to_screen(rig.target_x, cap_ground_y(), rig.target_z);
    if !p.visible { return; }
    let breathe = 1.0 + 0.08 * (total_time() * 2.0).sin();
    draw_circle_outline(p.x, p.y, 9.0 * breathe, 0.788, 0.635, 0.153, 0.85, 2.0);
    draw_circle(p.x, p.y, 3.0, 0.788, 0.635, 0.153, 1.0);
}

// ── The rig's geometry (pure) ────────────────────────────────────────────

// The camera for a focus point (fx, fz) on the ground, a zoom and a yaw:
// #{ px, py, pz } where it stands and #{ tx, ty, tz } what it looks at.
fn rig_pose(fx, fz, zoom, yaw) {
    let g = cap_ground_y();
    // Toward the map, look at the Capitol, north up.
    let c = smooth((zoom - centre_from()) / (1.0 - centre_from()));
    let lx = fx * (1.0 - c);
    let lz = fz * (1.0 - c);
    let n = smooth((zoom - north_from()) / (1.0 - north_from()));
    let heading = yaw - wrap_angle(yaw) * n;

    let d = rig_distance(zoom);
    let p = rig_pitch(zoom).to_radians();
    let across = d * p.cos();
    let px = lx + across * heading.sin();
    let pz = lz + across * heading.cos();
    let py = g + d * p.sin();

    // Stay out of the keep: slide out round it.
    let r = (px * px + pz * pz).sqrt();
    if r < keep_radius() && py < g + keep_top() {
        let u = if r > 0.001 { [px / r, pz / r] } else { [heading.sin(), heading.cos()] };
        px = u[0] * keep_radius();
        pz = u[1] * keep_radius();
        r = keep_radius();
    }
    // Rise over the curtain wall, and never touch the land.
    py = py.max(g + rig_floor(r));
    #{ px: px, py: py, pz: pz, tx: lx, ty: g, tz: lz }
}

// Distance from the focus (m) at a zoom.
fn rig_distance(zoom) {
    near_distance() * (far_distance() / near_distance()) ** zoom
}

// Tilt in degrees below level at a zoom: up from closest to street, held
// there, then easing to straight down. Smooth at the join.
fn rig_pitch(zoom) {
    let zs = street_zoom();
    if zoom <= zs {
        lerp(closest_pitch(), street_pitch(), smooth(zoom / zs))
    } else {
        lerp(street_pitch(), map_pitch(), smooth((zoom - zs) / (1.0 - zs)))
    }
}

// The lowest the camera may be above the land at horizontal radius r: over
// the bailey it clears the wall, eased out past the moat so crossing it is
// a rise, not a step.
fn rig_floor(r) {
    let inside = smooth((wall_outer() - r) / 2.0);
    ground_clear() + (wall_clear() - ground_clear()) * inside
}

fn smooth(t) {
    let t = t.max(0.0).min(1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a, b, t) { a + (b - a) * t }

// An angle brought into (-PI, PI].
fn wrap_angle(a) {
    let tau = 2.0 * PI();
    let w = (a + PI()) % tau;
    if w < 0.0 { w += tau; }
    w - PI()
}
