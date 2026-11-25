# BYTE artwork provenance

Original artwork for omniTerm. Created with the built-in `image_gen` tool, concept first, followed by a transparent sprite-sheet edit. No CLI fallback or external mascot assets were used. Project licensing is described in the repository's LICENSE and NOTICE.

## Concept prompt

Use case: stylized-concept. Create one beautiful polished original character design sheet for easter eggs in omniTerm, an engineering terminal desktop app. A warm little cream-colored terminal cat named BYTE, navy outline, rounded geometric silhouette, tiny dark navy ears, amber cheeks, blue terminal prompt >_ on a small bandana. Not an existing mascot. Make a cohesive editorial design board on warm off-white paper with generous margins, blue accent #3688F5 and apricot/coral. Header exactly 'omniTerm / little moments'. Three clearly separated horizontal panels across a landscape board: left a sleepy curled cat leaning on a tiny blue laptop and one z, center the same cat awake sitting, paw raised, a little heart, right the same cat jumping toward a simple blue packet-fish with a short dotted path and little sparkles. Below, a small tasteful inset of a dark navy rounded-square retro snake mini-game, blue snake and apricot packet-fish, no terminal text. Labels exactly 'REST', 'HELLO', 'PLAY' under the three character panels. Crisp charming high-end flat 2D illustration, subtle paper grain only background, no photorealism, no gradients on character, no complex fur, keep cat instantly recognizable and readable at small sizes. Design intended to guide animated native egui vector artwork. No screenshots, no copyrighted penguin or MobaXterm graphics, no logos besides the supplied text. This is a concept art board, not a website mockup.

## Final sprite-sheet prompt

Reference/edit target: the generated concept above. `transparent_background: true`.

Edit the displayed BYTE cat character concept sheet into a production sprite-sheet asset ONLY. Preserve precisely the same cream cat identity, thick navy outlines, tiny navy ears, orange cheeks, bright blue >_ bandana and charming flat style. Output a single TRANSPARENT PNG sheet with exactly THREE equally wide columns in one horizontal row, no background whatsoever, no labels, text, frames, shadows, grid, board, color swatches, footer or game UI. Keep a generous fully transparent gap between columns and transparent safety margin around each character. Each pose must be fully inside its own column and all three characters sit on the same baseline at matching body scale. Column one: sleeping cat curled up with little blue terminal laptop, like the REST art from the concept. Column two: seated awake cat waving a paw, like HELLO, little coral heart above its paw allowed. Column three: happy leaping cat catching ONE small blue packet-fish, like PLAY. Center the whole little scene within each equal column. Width to height ratio approximately 3:1. Render clean crisp solid-color 2D illustration suitable for a native desktop app, alpha-transparent empty space, no paper grain or white fill behind characters. No additional poses or subjects. Match the original design and make the three separate frames easy to crop in code at x = 0, 1/3, 2/3.

## Saved files

- `byte-concept.png`: 1536×1024 design board, preserved for review.
- `byte-sprites.png`: 2172×724 RGBA atlas; unchanged generated pixels. Runtime UV boundaries 0 / 0.335 / 0.62 / 1 select transparent gutters, preserving the leap's motion marks; the renderer fits the nontransparent bounds inside each frame without modifying the source PNG.
