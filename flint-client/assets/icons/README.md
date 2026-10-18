# Item icons

One SVG per item type, named for its `item_type` as the server sends it (`fiber.svg`, `chest_kit.svg`). The client loads every `*.svg` here at startup (`crates/capitol-client/src/ui/icons.rs`). So a new item type needs only a new file, and `every_item_in_the_catalog_has_its_own_icon` fails until it has one.

A type without a file is drawn with its catalog category's icon from `categories/` (`tool.svg`, `raw_material.svg`, ...): the category's shape in the fallback's slate, with a gold question tag at the lower left. A type the catalog doesn't know at all is drawn with `fallback.svg`.

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

**Tool tiers** read by material, on one drawing per kind (axe, pickaxe, hammer, sickle), leant 38° so the head is up and to the right. Stone is knapped flint, lashed to the haft in gold cord. Iron is dark iron (`#64748b` `#475569` `#334155`, ink `#0f172a`), its edge ground bright (`#f1f5f9`, a 1.6 line just inside the outline), and riveted in gold. A component is its tool's head alone, larger, with its eye showing and a gold maker's mark.

Palette so far: plant greens `#4d7c0f` `#65a30d` `#84cc16`; rope and wood `#b08850` `#a0742c` `#8b5a2b` `#6b4a1a`; iron `#64748b` `#94a3b8`; flint `#a8a29e` `#78716c` `#57534e` (ink `#1c1917`); heat `#f97316` `#fbbf24`; brick `#c2703f` `#9a4a2c` `#7a3820` (ink `#2a0f06`); clay `#a86d4f` `#8f5a40`; rust `#9a3412` `#b45309`; end grain and sawdust `#d9b98a`; charcoal `#3a3532` `#292524` `#1c1917`; unknown slate `#7a7a8a` `#6b6b7a` `#5a5a68` (ink `#15151c`).
