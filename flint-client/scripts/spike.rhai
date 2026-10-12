// P0.4 embedding spike (throwaway).
// Camera: south of the Capitol looking north, so east should be on the
// right. HUD: whatever is typed in the egui box, read through the native
// function cap_spike_note(), written over the Capitol.

fn on_update() {
    // A slow breath of a drift so the scene feels alive without hiding the
    // compass: the camera stays south of centre.
    let t = total_time();
    let sway = 6.0 * (t * 0.15).sin();
    set_camera_position(sway, 70.0, 95.0);
    set_camera_target(0.0, 0.0, 4.0);
}

fn label(x, z, text, size, r, g, b) {
    let p = world_to_screen(x, 0.5, z);
    if !p.visible { return; }
    let m = measure_text(text, size);
    let tx = p.x - m.width / 2.0;
    let ty = p.y - m.height / 2.0;
    draw_text(tx + 1.5, ty + 1.5, text, size, 0.0, 0.0, 0.0, 0.6);
    draw_text(tx, ty, text, size, r, g, b, 1.0);
}

fn on_draw_ui() {
    // Compass labels sit beside the markers (server radius 400 = 40 m).
    label(46.0, 0.0, "E", 18.0, 0.976, 0.451, 0.086);
    label(0.0, -46.0, "N", 18.0, 0.878, 0.878, 0.878);

    let note = cap_spike_note();
    if note != "" {
        label(0.0, 0.0, note, 30.0, 0.788, 0.635, 0.153);
    }
}
