# Sound credits

Every sound in The Capitol's Flint client is synthesized from noise and sine
waves by `crates/capitol-client/src/sound/library.rs`. No recordings and no
third-party samples are used. The client makes them in memory when it starts.
`capitol --write-sounds <dir>` writes them out as `.wav` files to listen to.

The sounds, like the code that makes them, are dedicated to the public domain
under [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/).

| File | What it is | How it's made |
|---|---|---|
| `amb_city.wav` | City hum at the core | Brown-noise traffic rumble, a band-passed pink-noise crowd murmur, faint air, and a drone on A (55 Hz, its octave and fifth) |
| `amb_breeze.wav` | Suburbs and fields | Low-passed pink-noise air, leaf crackle that rises with the gusts, a low hush |
| `amb_wind.wav` | Wilderness, and high over the map | Pink noise through two resonant bands that ride the gusts, over a brown-noise buffet |
| `amb_water.wav` | Water lapping at the rings | A brown-noise wash, laps that open a low-pass as they break, sparse bubbles |
| `ui_click.wav`, `ui_drop.wav` | Buttons, and an item set down in a slot | Short damped sines with a noise transient |
| `rustle_1..3.wav` | Grass being gathered | Band-passed noise grains |
| `pluck.wav` | A gather lands | Two Karplus-Strong plucks, G5 then D6 |
| `chime.wav` | A craft completes | A D-major arpeggio on struck bars (free-bar partials), with reverb |
| `bell.wav` | The tower's bell, for a claimed deed and now and then near the core | A minor-third church bell on G3, detuned partial pairs, distant reverb |
| `bird_1..4.wav` | Songbirds in the suburbs and fields | Gliding sine chirps with vibrato |
