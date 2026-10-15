# Item icons

One SVG per item type, named for its `item_type` as the server sends it (`fiber.svg`, `chest_kit.svg`). The client loads every `*.svg` here at startup (`crates/capitol-client/src/ui/icons.rs`). A type without a file is drawn with `fallback.svg`. So a new item type needs only a new file.

To review them all on the dark panel, from `flint-client/`:

```
target/release/capitol.exe --offline fixtures/terrain.json --icon-sheet --screenshot-at 1 --exit-after 2 --screenshot renders/icons.png
```

## The style

Keep new icons in this style so the inventory reads as one set:

- **Canvas:** `viewBox="0 0 64 64"`. Keep the subject inside x 8–56 and y 2–58. The slot draws them at 36 points, with a grade pip in the top right and the quantity in the bottom right, so don't put detail in those corners.
- **Ground:** a soft shadow under each subject, `<ellipse cx="32" cy="56.5" ry="3.2" fill="#000" fill-opacity="0.35"/>`, with `rx` matching the subject's width.
- **Fills:** warm and flat, two or three tones of one material, with the darker tone underneath or in front.
- **Ink:** a 1.5-unit outline in a near-black of the material's hue (`#1b1206` for wood and rope, `#0f172a` for iron, `#14240a` for plants), with round joins.
- **Light:** from the top left. One thin highlight line on the top edge, a pale tint at 40–60% opacity.
- **Gold:** one touch of the Capitol's gold `#c9a227`: a band, a lock, a tie. It makes them read as one family on the panel.

Palette so far: plant greens `#4d7c0f` `#65a30d` `#84cc16`; rope and wood `#b08850` `#a0742c` `#8b5a2b` `#6b4a1a`; iron `#64748b` `#94a3b8`; heat `#f97316` `#fbbf24`; unknown slate `#6b6b7a`.
