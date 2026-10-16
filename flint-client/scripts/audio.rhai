// What you hear (P8.1). The extension decides every sound (crate::sound):
// the ambience beds it mixes itself, and each one-shot it fires here as a
// `sound` event, because only scripts can reach Flint's audio engine.
//
// data: name ("audio/<file>.wav"), volume, pitch, and placed with x, y, z
// (world metres, a few metres from the camera in the sound's direction,
// so it pans). Unplaced sounds have no direction and no pitch.

fn on_event(name, data) {
    if name != "sound" { return; }
    if data.placed {
        play_sound_at(data.name, data.x, data.y, data.z, data.volume, data.pitch);
    } else {
        play_sound(data.name, data.volume);
    }
}
