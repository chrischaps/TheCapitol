// The camera rig (P3.9). The camera follows a point on the ground with
// damping. Zoom runs on one dial, from standing over someone's shoulder to
// the whole world seen from above, and the tilt rides on it:
//
//   - the shoulder (E3.13, 4.5 m off): nearly level, 2 m up, about 3 m
//     behind you and a little to your right, looking past you at chest
//     height, so the street ahead fills the frame;
//   - 11 m off: 15° down and under 3 m up, standing in the street with the
//     sky in the top third of the frame, and from the outer zones the
//     tower on the horizon;
//   - street (32 m): about 50° down, where the camera rests;
//   - map (1200 m): straight down, north at the top, the whole r8300
//     world in frame, the light flattened into the 2D client's palette
//     (atmosphere.rs keys that on height).
//
// In the wilds (W9) the rig follows the land: it looks at the ground under
// the point (cap_ground_at), keeps above the land under itself, and rises
// so that no hill stands between it and the point.
//
// On the way to the map the view drifts to the Capitol and turns north up,
// so the furthest zoom is always the same map.
//
// Close in (E3.13), the camera keeps out of the buildings: when a wall
// stands between you and it (clear_line: the buildings are drawn as ray
// targets, and raycast_meshes meets their walls and roofs, #293), it rises
// at once, over the roofs behind you, and looks down into the street; if
// rising isn't enough, it comes in along its line to you. It eases back
// as the way clears. And while you walk, close in, it swings round behind
// you, so the street ahead is what you see, unless you've just turned it
// yourself.
// Walking into the bazaar (cap_in_bazaar) eases the camera down to the
// shoulder, so the market street fills the view, and walking out eases it
// back to where you had it, unless you turned the wheel meanwhile.
//
// Controls (config/input.toml): WASD or the arrows pan, R and F, + and -
// or the wheel zoom, Q and E or a right-drag orbit, H or Home comes home.
// The wheel is the `zoom` action, in notches.
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
// `--view` (cap_view) frames a capture. street, district, map and bazaar
// (the free point in the bazaar's lane, E3.13) are rig framings. The rest are fixed shots: bridge (the quay by the inner canal's
// south crossing), suburb (low over the southern suburbs' curving streets),
// urban (close over the lots by the south arterial), capitol (up at the
// tower from beyond the moat), crown (level with its lantern), and
// from-urban, from-suburban and from-rural (the tower on the horizon from
// each outer zone), from-wilderness (down from a ridge over the valleys to
// the tower, W11) and range (out along the ranges from a summit, W11). The
// wilds' two stand on the default world's land. A shot holds until the first camera
// input, then eases into the rig, which takes up what the shot was looking
// at from about as far off, so the subject stays in frame.
//
// `--camera-path` (cap_camera_path, issue #92) films a move offline: a
// moving shot, the camera wherever the path's keys put it at that moment.
// It holds at its last key, and lets go into the rig like any shot.
//
// The player's settings (P8.4) scale how fast it turns and zooms
// (cap_setting("camera_sensitivity")) and can turn the orbit the other way
// (cap_setting("invert_orbit")). The rates below are the rig as made.
// Switching back to point and walk (F8, cap_setting("control_mode"), T1.1)
// drops the pan, so the view comes back onto you.
//
// Over the shoulder (Direct, T1.3), WASD walks you rather than panning the
// view: Rust reads the keys and walks against the yaw the rig means to
// have, which it says each frame with cap_view_yaw (not the camera's, which
// drifts round the keep). So the keys don't pan, Home doesn't reset a pan,
// Q/E don't orbit (E acts, T1.5), and walking doesn't swing the view round
// behind you, which would turn the way W walks as you went.
//
// rig_pose and the functions under it are pure, and a Rust test
// (camera_rig.rs) drives them across every zoom, yaw and position.

// ── Tuning ───────────────────────────────────────────────────────────────

// Camera distance from the point it looks at (m) at zoom 0 and zoom 1.
// Zoom is linear in the log of distance, so each wheel notch feels alike.
fn near_distance() { 4.5 }
// The far end frames the whole world at the rig's 70° (830 m, r8300, since
// Plots v2's flip grew it, #459; 1000 m framed r6800, and 880 m r6000).
fn far_distance() { 1200.0 }
// The shoulder view (E3.13) fades in from this distance down to the near
// end: the look rises to chest height, moves ahead of you and over your
// right shoulder (m). 11 m is where the dial used to end.
fn shoulder_distance() { 11.0 }
fn shoulder_zoom() { zoom_at(shoulder_distance()) }
fn look_lift() { 1.6 }
fn look_ahead() { 1.5 }
fn shoulder_side() { 0.45 }
// Close in, a wall between you and the camera pulls it in (E3.13): below
// this distance, never nearer than this to what it looks at (m), easing
// back out at this rate.
fn pull_within() { 14.0 }
fn pull_least() { 1.2 }
fn pull_k() { 3.0 }
// How far the camera may rise over the roofs before it pulls in instead,
// and the steps it tries (m).
fn rise_most() { 3.0 }
fn rise_step() { 0.25 }
// Close in, walking swings the camera round behind you: from this pace up
// (m/s), at this rate, and not for this long after you've turned it (s).
fn follow_from() { 0.6 }
fn follow_k_yaw() { 1.4 }
fn follow_after_turn() { 1.5 }
// How fast the zoom eases when the bazaar takes it down or gives it back:
// slower than a wheel's, an arrival rather than a jump.
fn arrive_zoom_k() { 1.6 }
// Where the camera rests: 32 m off, 50° down, about 25 m up.
fn street_distance() { 32.0 }
fn street_zoom() {
    (street_distance() / near_distance()).ln() / (far_distance() / near_distance()).ln()
}
// The district view stands 232 m off, as it did when the map was 640 m
// up (before W9 grew it to the whole world).
fn district_distance() { 232.0 }
fn district_zoom() { zoom_at(district_distance()) }
// Tilt (degrees below level) at zoom 0, at the shoulder's edge (11 m), at
// street and at the map. Just short of 90 at the map, so the look-at keeps
// a heading: 89.92, so from 1200 m up it's still under 2 m off straight
// down (89.9 from 1000 m before #459).
fn closest_pitch() { 5.0 }
fn shoulder_pitch() { 15.0 }
fn street_pitch() { 50.0 }
fn map_pitch() { 89.92 }
// The view drifts from the point to the Capitol from this far out (m), and
// turns north up from the second. They're distances, so moving the dial's
// near end (E3.13) doesn't move them.
fn centre_from() { zoom_at(166.5) }
fn north_from() { zoom_at(366.5) }
// Keep the free point on the world (r6800, 680 m).
fn world_radius() { cap_world_radius() }
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
// A notch of the wheel multiplies the distance by about 1.22, as it did
// before the dial reached the shoulder (E3.13): the steps are in zoom, so
// they shrink as the dial grows.
fn wheel_step() { 0.0374 }   // zoom per wheel notch
fn key_zoom_rate() { 0.374 } // zoom per second on R/F or +/-
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
fn clear_ramp() { 6.0 }

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
    // The shot is a --camera-path being played.
    path: false,
    started: false,
    // The login circle held last frame, and how long it has waited past
    // the form for where you are.
    at_login: false, waited: 0.0,
    // How far the view is panned from your avatar (m). Unused offline.
    pan_x: 0.0, pan_z: 0.0,
    // The control mode last frame, "click" or "direct" (T1.1).
    mode: (),
    // A canal stopped you (P4.3): #{ born, dx, dz }, the way you were
    // going, for a nudge toward the water and back. () when still.
    bump: (),
    // The zone last seen in the banner, and when you walked into it (P8.2).
    zone: (), arrived: (),
    // Close in, how far out along its line the camera may stand, 0 to 1:
    // a wall pulls it in at once, and it eases back (E3.13).
    pull: 1.0,
    // How far it has risen over the roofs (m), and the wall clock of your
    // last turn of the camera, and where you were last frame.
    rise: 0.0, turned: -100.0, last_you: (),
    // In the bazaar (E3.13): whether you are, the zoom you had before it
    // took you down, whether you've zoomed since, and whether the zoom is
    // easing for it (slower than a wheel).
    in_bazaar: false, before: (), zoomed: false, arriving: false,
};

// ── Per frame ────────────────────────────────────────────────────────────

fn on_update() {
    // A hitch mustn't fling the camera across the city.
    let dt = delta_time().min(0.1);
    if !rig.started {
        rig.started = true;
        rig.start_view(cap_view());
    }

    let mode = cap_setting("control_mode");
    if rig.mode == "direct" && mode == "click" {
        rig.pan_x = 0.0;
        rig.pan_z = 0.0;
    }
    rig.mode = mode;

    let moved = false;
    let you = cap_follow();
    let waiting = !cap_at_login() && rig.at_login && you == () && rig.waited < landing_wait();
    if cap_at_login() || waiting {
        rig.waited = if waiting { rig.waited + dt } else { 0.0 };
        rig.shot = login_shot(total_time());
        rig.release = 1.0;
        rig.at_login = true;
    } else if rig.at_login {
        rig.at_login = false;
        rig.waited = 0.0;
        rig.take_over_from_shot(false);
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
        // Close in, walking swings the camera round behind you (E3.13).
        // Not over the shoulder, where W walks the way the view faces.
        if rig.last_you != () && dt > 0.0 && rig.mode != "direct" {
            let vx = (you[0] - rig.last_you[0]) / dt;
            let vz = (you[2] - rig.last_you[2]) / dt;
            rig.yaw_to = follow_yaw(rig.yaw_to, vx, vz, rig.zoom, total_time() - rig.turned, dt);
        }
        rig.last_you = you;
    }
    // The bazaar takes the view down to the shoulder (E3.13): you, or
    // offline the free point, when the rig is driving
    if !rig.at_login && rig.shot == () {
        let at = if you != () { [you[0], you[2]] } else { [rig.target_x, rig.target_z] };
        rig.bazaar_ease(cap_in_bazaar(at[0], at[1]));
    }
    if rig.path {
        if moved {
            rig.path = false;
        } else {
            let p = cap_camera_path(total_time());
            rig.shot = #{ pos: p.pos, at: p.at };
            if p.done { rig.path = false; }
        }
    }
    if moved && rig.shot != () && rig.release >= 1.0 {
        rig.take_over_from_shot(true);
    }

    // Damping: exponential, so it's the same at any frame rate.
    let f = 1.0 - (-follow_k() * dt).exp();
    rig.focus_x += (rig.target_x - rig.focus_x) * f;
    rig.focus_z += (rig.target_z - rig.focus_z) * f;
    let zk = if rig.arriving { arrive_zoom_k() } else { zoom_k() };
    rig.zoom += (rig.zoom_to - rig.zoom) * (1.0 - (-zk * dt).exp());
    if (rig.zoom_to - rig.zoom).abs() < 0.002 { rig.arriving = false; }
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
    // Close in, keep out of the buildings: up (or in) at once, back at an
    // ease
    if rig.shot == () && rig_distance(rig.zoom) <= pull_within() {
        // From your head, so a wall you face isn't taken for one behind
        let head = [rig.focus_x, cap_ground_at(rig.focus_x, rig.focus_z) + look_lift(), rig.focus_z];
        let need = rise_needed(pose, head);
        let ease = 1.0 - (-pull_k() * dt).exp();
        if need > rig.rise { rig.rise = need; } else { rig.rise += (need - rig.rise) * ease; }
        pose = lifted(pose, rig.rise);
        let clear = clear_share(pose, rig_distance(rig.zoom), head);
        if clear < rig.pull { rig.pull = clear; } else { rig.pull += (clear - rig.pull) * ease; }
        pose = pulled(pose, rig.pull);
    } else {
        rig.pull = 1.0;
        rig.rise = 0.0;
    }
    rig.watch_zone();
    if rig.shot == () { pose = rig.eased_in(pose); }
    pose = rig.nudged(pose);
    set_camera_position(pose.px, pose.py, pose.pz);
    set_camera_target(pose.tx, pose.ty, pose.tz);
    cap_view_yaw(rig.yaw_to);
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

// The bazaar's ease (E3.13), with `inside` whether you're in it now. Coming
// in, the zoom eases down to the shoulder, unless it's there already, and
// what you had is kept; going out, it eases back to that, unless you've
// zoomed meanwhile, which counts.
fn bazaar_ease(inside) {
    if inside && !this.in_bazaar {
        this.zoomed = false;
        if this.zoom_to > shoulder_zoom() * 0.5 {
            this.before = this.zoom_to;
            this.zoom_to = 0.0;
            this.arriving = true;
        } else {
            this.before = ();
        }
    } else if !inside && this.in_bazaar {
        if this.before != () && !this.zoomed {
            this.zoom_to = this.before;
            this.arriving = true;
        }
        this.before = ();
    }
    this.in_bazaar = inside;
}

// The yaw to aim for, from `yaw`, while you move at (vx, vz) m/s: close in
// (by the zoom), eased round toward behind you, the short way, unless you
// turned the camera within follow_after_turn() (`since` seconds ago).
fn follow_yaw(yaw, vx, vz, zoom, since, dt) {
    let k = 1.0 - smooth(zoom / shoulder_zoom());
    let pace = (vx * vx + vz * vz).sqrt();
    if k <= 0.0 || pace < follow_from() || since < follow_after_turn() { return yaw; }
    // Behind you, looking the way you go (as take_over_from_shot reads a
    // look direction)
    let behind = atan(-vx, -vz);
    let turn = wrap_angle(behind - yaw);
    yaw + turn * k * (1.0 - (-follow_k_yaw() * dt).exp())
}

// How far the camera must rise for its line to your `head` to clear the
// buildings, in rise_step()s up to rise_most(); rise_most() if no height
// clears it (then it pulls in too).
fn rise_needed(pose, head) {
    let h = 0.0;
    while h <= rise_most() {
        let clear = clear_line(head, [pose.px, pose.py + h, pose.pz]);
        if clear >= 1.0 { return h; }
        h += rise_step();
    }
    rise_most()
}

// The pose with the camera `h` m higher, looking at the same point.
fn lifted(pose, h) {
    if h <= 0.0 { return pose; }
    #{ px: pose.px, py: pose.py + h, pz: pose.pz, tx: pose.tx, ty: pose.ty, tz: pose.tz }
}

// What share of the way from your `head` out to the camera is clear of the
// buildings (clear_line): 1 unless the camera is close in.
fn clear_share(pose, d, head) {
    if d > pull_within() { return 1.0; }
    clear_line(head, [pose.px, pose.py, pose.pz])
}

// How far, as a share of the way from `a` to `b` (world points), the line
// runs before it meets a building's wall or roof: 1 when it's clear. The
// buildings are the meshes the city draws as ray targets (the stalls, the
// hall, the houses), where the last frame drew them (Flint ADR 0105).
fn clear_line(a, b) {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let dz = b[2] - a[2];
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if len <= 0.0 { return 1.0; }
    let hit = raycast_meshes(a[0], a[1], a[2], dx, dy, dz, len);
    if hit == () { 1.0 } else { hit.distance / len }
}

// The pose with the camera brought in along its line to what it looks at,
// to `share` of the way out, but never nearer than pull_least().
fn pulled(pose, share) {
    if share >= 1.0 { return pose; }
    let dx = pose.px - pose.tx;
    let dy = pose.py - pose.ty;
    let dz = pose.pz - pose.tz;
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if len < 0.001 { return pose; }
    let k = share.max(pull_least() / len).min(1.0);
    #{ px: pose.tx + dx * k, py: pose.ty + dy * k, pz: pose.tz + dz * k,
       tx: pose.tx, ty: pose.ty, tz: pose.tz }
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
    let k = arrive_size() * e * (1.0 - smooth((this.zoom - centre_from()) / 0.125));
    #{ px: pose.px + (pose.px - pose.tx) * k, py: pose.py + (pose.py - pose.ty) * k,
       pz: pose.pz + (pose.pz - pose.tz) * k, tx: pose.tx, ty: pose.ty, tz: pose.tz }
}

// Read this frame's camera input into the rig (`this`). Returns whether
// there was any, which is what lets go of a fixed shot. `you` is
// cap_follow(): with an avatar, WASD moves the pan from it, or over the
// shoulder walks you (T1.3), which lets go of a shot too.
fn steer(dt, you) {
    let any = false;
    let direct = this.mode == "direct";

    // Move the point, relative to the way the camera faces. Faster the
    // further out the camera is, so a screen's width takes the same time.
    let fwd = action_axis("move_forward", "move_backward");
    let side = action_axis("move_right", "move_left");
    if (fwd != 0.0 || side != 0.0) && direct && you != () {
        any = true;
    } else if fwd != 0.0 || side != 0.0 {
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
    let notches = action_value("zoom");
    let keys = action_axis("zoom_out", "zoom_in");
    if notches != 0.0 || keys != 0.0 {
        any = true;
        this.zoomed = true;
        this.arriving = false;
        this.zoom_to = (this.zoom_to - (notches * wheel_step() - keys * key_zoom_rate() * dt) * feel)
            .max(0.0).min(1.0);
    }

    // Orbit: Q turns the view left, E right, and so does dragging right.
    // Over the shoulder only the drag does.
    let turn = if direct { 0.0 } else { action_axis("orbit_left", "orbit_right") };
    if turn != 0.0 {
        any = true;
        this.turned = total_time();
        this.yaw_to += turn * orbit_rate() * dt * feel * way;
    }
    if is_mouse_pressed(1) {
        let x = mouse_x();
        if this.drag_x != () && x != this.drag_x {
            any = true;
            this.turned = total_time();
            this.yaw_to -= (x - this.drag_x) * drag_rate() * feel * way;
        }
        this.drag_x = x;
    } else {
        this.drag_x = ();
    }

    // Home: back to the start, at street zoom, north up. The yaw unwinds
    // by the short way, never a full turn.
    // Over the shoulder there's no pan to drop.
    if is_action_just_pressed("home") {
        any = true;
        if !direct || you == () {
            let home = follow_home(you);
            this.target_x = home[0];
            this.target_z = home[1];
            this.pan_x = 0.0;
            this.pan_z = 0.0;
        }
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
    } else if view == "bazaar" {
        // In the bazaar's lane (E3.12), looking west up it from the mouth:
        // the bazaar then takes the rig down to the shoulder (E3.13)
        at = [-6.0, 29.75];
        this.yaw = PI() / 2.0;
        this.yaw_to = this.yaw;
    } else {
        this.shot = view_shot(view);
        this.release = 1.0;
    }
    let path = cap_camera_path(0.0);
    if path != () {
        this.shot = #{ pos: path.pos, at: path.at };
        this.release = 1.0;
        this.path = true;
    }
    if at != () {
        this.target_x = at[0];
        this.target_z = at[1];
    }
    this.zoom_to = this.zoom;
    this.focus_x = this.target_x;
    this.focus_z = this.target_z;
}

// The shot hands over to the rig, facing the way the shot faced. Then the
// camera eases across from the shot.
//
// `framed`: the rig takes up what the shot was looking at, the ground under
// it, from about as far off, or further if that's what it takes to keep a
// subject up in the air (the crown) in the middle of the frame. No wider
// than the district, so a shot from the horizon doesn't crane to the map.
// So the subject stays in frame, and Q/E or a drag then orbit round it
// (issue #92). Otherwise (the
// login circle, which lands on you next) it's street zoom, with the point
// followed just ahead of the camera.
fn take_over_from_shot(framed) {
    let s = this.shot;
    let dx = s.at[0] - s.pos[0];
    let dy = s.at[1] - s.pos[1];
    let dz = s.at[2] - s.pos[2];
    let far = (dx * dx + dy * dy + dz * dz).sqrt();
    let len = (dx * dx + dz * dz).sqrt();
    if len < 0.001 {
        // Straight down: north up.
        dx = 0.0;
        dz = -1.0;
        len = 1.0;
    }
    this.yaw = atan(-dx, -dz);
    if framed {
        this.target_x = s.at[0];
        this.target_z = s.at[2];
        this.zoom = framing_zoom(s.at, zoom_at(far), this.yaw);
    } else {
        let ahead = rig_distance(street_zoom()) * street_pitch().to_radians().cos();
        this.target_x = s.pos[0] + dx / len * ahead;
        this.target_z = s.pos[2] + dz / len * ahead;
        this.zoom = street_zoom();
    }
    this.focus_x = this.target_x;
    this.focus_z = this.target_z;
    this.yaw_to = this.yaw;
    this.zoom_to = this.zoom;
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
        // Low over the neighbourhood west of the south arterial (#112): its
        // warped rows and their bulbs sweep in towards the tower.
        #{ pos: [-30.0, 26.0, 300.0], at: [-50.0, 0.0, 232.0] }
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
        // A sixth of the way into the suburb (r2400 since Plots v2, #459).
        #{ pos: [170.0, 16.0, 170.0], at: [0.0, 30.0, 0.0] }
    } else if view == "from-rural" {
        // Out in the middle of the rural ring (r4850 since Plots v2, #459),
        // between its second and third main lanes, the countryside running
        // in to the city.
        #{ pos: [242.5, 22.0, -419.9], at: [0.0, 30.0, 0.0] }
    } else if view == "range" {
        // Over the southern massif's summit (61 m, r7720 since Plots v2,
        // #459), looking east along the ring: down into the south valley and
        // its trail, and across to the next massif's tops.
        #{ pos: [-285.4, 75.9, 717.3], at: [205.6, 30.0, 719.9] }
    } else {
        // from-wilderness: on a ridge of the north-west massif (66 m, r7805
        // since Plots v2, #459), over its valleys and the plain to the
        // tower, gold at the centre.
        #{ pos: [-298.7, 69.2, -721.1], at: [0.0, 22.0, 0.0] }
    }
}

// Offline, the free point, marked in gold: gold means you. A dot and a
// ring on the screen, a constant size at every zoom. Signed in, your
// avatar carries the gold instead (entities.rs).
fn on_draw_ui() {
    if cap_follow() != () || cap_at_login() || cap_clean() { return; }
    if rig.shot != () && rig.release >= 1.0 { return; }
    let p = world_to_screen(rig.target_x, cap_ground_at(rig.target_x, rig.target_z), rig.target_z);
    if !p.visible { return; }
    let breathe = 1.0 + 0.08 * (total_time() * 2.0).sin();
    draw_circle_outline(p.x, p.y, 9.0 * breathe, 0.788, 0.635, 0.153, 0.85, 2.0);
    draw_circle(p.x, p.y, 3.0, 0.788, 0.635, 0.153, 1.0);
}

// ── The rig's geometry (pure) ────────────────────────────────────────────

// The camera for a focus point (fx, fz) on the ground, a zoom and a yaw:
// #{ px, py, pz } where it stands and #{ tx, ty, tz } what it looks at.
fn rig_pose(fx, fz, zoom, yaw) {
    // Toward the map, look at the Capitol, north up.
    let c = smooth((zoom - centre_from()) / (1.0 - centre_from()));
    let lx = fx * (1.0 - c);
    let lz = fz * (1.0 - c);
    // The ground under what it looks at: the land in the wilds.
    let g = cap_ground_at(lx, lz);
    let n = smooth((zoom - north_from()) / (1.0 - north_from()));
    let heading = yaw - wrap_angle(yaw) * n;

    let d = rig_distance(zoom);
    let p = rig_pitch(zoom).to_radians();
    // Close in (E3.13), the look rises to chest height and moves ahead of
    // the point and over its right shoulder. Away from you is
    // (-sin, -cos) in x, z, and right is (cos, -sin).
    let k = 1.0 - smooth(zoom / shoulder_zoom());
    let sh = heading.sin();
    let ch = heading.cos();
    lx += (-sh * look_ahead() + ch * shoulder_side()) * k;
    lz += (-ch * look_ahead() - sh * shoulder_side()) * k;
    let ly = g + look_lift() * k;
    let across = d * p.cos();
    let px = lx + across * sh;
    let pz = lz + across * ch;
    let py = ly + d * p.sin();

    // Stay out of the keep: slide out round it.
    let r = (px * px + pz * pz).sqrt();
    if r < keep_radius() && py < g + keep_top() {
        let u = if r > 0.001 { [px / r, pz / r] } else { [heading.sin(), heading.cos()] };
        px = u[0] * keep_radius();
        pz = u[1] * keep_radius();
        r = keep_radius();
    }
    // Rise over the curtain wall, and never touch the land.
    py = py.max(cap_ground_at(px, pz) + rig_floor(r));
    py = over_the_land(px, py, pz, lx, ly, lz);
    #{ px: px, py: py, pz: pz, tx: lx, ty: ly, tz: lz }
}

// The least height, from `py` up, at which the camera at (px, pz) sees the
// point (tx, ty, tz) over the land: its line clears the ground under it
// every few metres along the way (at most 64 places), by ground_clear(),
// eased in over the first clear_ramp() metres from the point, which is on
// the ground itself. A full clearance right up to it held the near end
// metres too high (E3.13).
fn over_the_land(px, py, pz, tx, ty, tz) {
    let len = ((px - tx) * (px - tx) + (pz - tz) * (pz - tz)).sqrt();
    if len < 0.01 { return py; }
    let steps = (len / 2.5).ceiling().to_int().max(4).min(64);
    // Every half metre where the clearance eases in, then every few
    let ts = [];
    for k in 1..13 {
        let d = k.to_float() * 0.5;
        if d < len { ts.push(d / len); }
    }
    for k in 1..steps { ts.push(k.to_float() / steps.to_float()); }
    for t in ts {
        let h = cap_ground_at(tx + (px - tx) * t, tz + (pz - tz) * t) + ground_clear() * (len * t / clear_ramp()).min(1.0);
        // The line is at ty + (py - ty) t there.
        let need = ty + (h - ty) / t;
        if need > py { py = need; }
    }
    py
}

// Distance from the focus (m) at a zoom.
fn rig_distance(zoom) {
    near_distance() * (far_distance() / near_distance()) ** zoom
}

// The least zoom from `from` on, up to the district's, at which the rig
// looking at the ground under `at` (facing `yaw`) has `at` itself within
// `framing_angle` of the middle of the frame.
fn framing_zoom(at, from, yaw) {
    let z = from.min(district_zoom());
    while z < district_zoom() {
        let pose = rig_pose(at[0], at[2], z, yaw);
        let lx = pose.tx - pose.px;
        let ly = pose.ty - pose.py;
        let lz = pose.tz - pose.pz;
        let sx = at[0] - pose.px;
        let sy = at[1] - pose.py;
        let sz = at[2] - pose.pz;
        let cos = (lx * sx + ly * sy + lz * sz)
            / ((lx * lx + ly * ly + lz * lz) * (sx * sx + sy * sy + sz * sz)).sqrt();
        if cos >= framing_angle().to_radians().cos() { return z; }
        z += 0.01;
    }
    district_zoom()
}
// Off the middle of the frame (degrees) a shot's subject may be once the
// rig has it: well inside the 35° to the frame's top and bottom.
fn framing_angle() { 20.0 }

// The zoom that stands the camera `d` m from the focus.
fn zoom_at(d) {
    ((d / near_distance()).ln() / (far_distance() / near_distance()).ln()).max(0.0).min(1.0)
}

// Tilt in degrees below level at a zoom: nearly level at the shoulder, up
// to 11 m, on up to street, then easing to straight down. Smooth at the
// joins.
fn rig_pitch(zoom) {
    let zs = street_zoom();
    let zh = shoulder_zoom();
    if zoom <= zh {
        lerp(closest_pitch(), shoulder_pitch(), smooth(zoom / zh))
    } else if zoom <= zs {
        lerp(shoulder_pitch(), street_pitch(), smooth((zoom - zh) / (zs - zh)))
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
