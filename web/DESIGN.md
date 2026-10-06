# Lentic design direction

Every loan is a gauge station on still water. Read this before adding a page or a component. If the work needs
something this document does not allow, change the document first and give the reason. `pnpm design:check` enforces
the bans below.

## Product world

- **Does:** lends USDC from a lender's own Base vault against ZEC that stays in a 2-of-3 shielded escrow (borrower,
  lender, policy signer). The policy signer reads the price every round and reports the loan's health to LoanBook.
- **First ten seconds:** a lender wants to know which loans are near liquidation; a borrower wants to know how far the
  price can fall before the collateral can be taken.
- **Proof data:** each round's reading: ZEC/USD (median of five exchanges), accrued debt, committed collateral and the
  resulting loan-to-value, plus the health reports the attested policy signer wrote to LoanBook.
- **Worst moment:** liquidation. LTV reaches 75 % and the lender and the policy signer may sign the escrow spend.
- **Never confuse:** margin call (a warning, nothing moves) and liquidation (collateral can move); who can move the
  collateral (any two of three keys, never one, never Lentic alone); what is public and what only the parties see.
- **Name:** lentic, the ecology of still water. The ZEC stays in the pool.

## Neighbours surveyed

Surveyed 6 Oct 2026 from each site's HTML and stylesheets (the in-app browser could not open external sites, so
signature elements come from markup and copy, not screenshots).

| Site | Signature element | Display type and weight | Colours | Real data |
|---|---|---|---|---|
| z.cash | Price and shielded-supply figures right under the hero | DM Serif Display with Inter, 300 to 500 | White, black, Zcash yellow `#F3B724`, cyan `#00B2FF` | ZEC price, supply, shielded ZEC |
| Zodl | Phone screenshot of the wallet | Michroma (wide) with Anek Latin | Warm blacks and greys | None |
| Colosseum | Illustrated arena, Old London initials | GT Pantheon Text with Favorit Mono, 400 to 500 | Greys, one deep red `#C00000` | Builder counts |
| Base | Partner logo carousel, the blue square | Base Sans, 100 to 500 | `#0000FF` with pastels | Network totals |
| Unchained | Lifestyle photo with a phone widget | IvyPresto headline serif with Inter | Navy `#01121E`, blue | 14.18 % APR, 200 % collateral |
| Firefish | Four-box statistics row | Satoshi and Inter, 500 to 700 | Near-black, red `#E6413E` | Market statistics |
| Morpho | Wireframe line art, exact totals | FK Grotesk, 300 to 500 | White, black, one blue | Deposits |
| Phala | Near-black with acid green | Serif with grotesk | `#070908`, `#C5FF2E` | None |
| USGS Water Data | Hydrographs and gauge readings in plain tables | Source Sans with Merriweather | Federal navy and blue | Live gauge heights |

Shared baseline: light display type (300 to 500), one brand colour used sparingly, real numbers.
Nobody does: draw one loan's live position against its thresholds, or show which keys can move the money.

## Concept

Every loan is a gauge station. The water line on a staff gauge is the loan-to-value ratio.

| Product fact | Visual |
|---|---|
| Loan | A station, named by its loan id |
| Loan-to-value | The water line on a vertical staff gauge, 0 to 100 % |
| Margin call threshold (60 %) | Action stage: a ruled ink mark on the staff |
| Liquidation threshold (75 %) | Flood stage: the only coloured mark on the staff |
| Price falls | Water rises; the line moves up |
| Policy signer rounds | The hydrograph: LTV over time with both stages drawn across it |
| Collateral in the shielded escrow | The water itself, a flat lake tint: visible to the parties, never touchable |
| 2-of-3 FROST | The key-turn ledger: every event shows which of the three keys acted |
| How a liquidation happens (home page) | The flood drill: a live station whose price is lowered in simulated rounds until two keys turn and the ZEC is split |

Tests:

- **Portability: pass.** Stages, a rising line and a flood mark only make sense for a collateral ratio with fixed
  thresholds; a payments or trading product has no water level.
- **Data: pass.** The water line, the hydrograph and the ledger are drawn from readings and Base events. Nothing is
  illustrated.
- **Failure: pass.** Liquidation is the water over flood stage: the line, the reading and the stage sentence turn
  flood red, and nothing else on the page is red.
- **Squint: pass.** At 20 % the page is a tall ruled staff with a flat tint, unlike photo heroes, logo carousels and
  stat boxes.
- **Explainability: pass.** One caption: "Water line = loan-to-value. Action stage = margin call. Flood stage =
  liquidation."

## Palette

| Name | Hex | Role |
|---|---|---|
| Paper | `#F6F7F5` | Page background. Cool survey white, not cream |
| Sheet | `#FFFFFF` | The staff, tables and inputs |
| Ink | `#16211D` | Text, ticks, action stage, primary buttons |
| Graphite | `#56625C` / `#7F8984` | Secondary text; the lighter tone only for large labels and ticks (3.4:1) |
| Rule | `#D3DAD6` | Hairlines, table rules, borders |
| Lake | `#D2DFD8` / `#3D6A5C` | The shielded pool: water fill and the water line. Never used for text on lake |
| Flood | `#BF3A1F` | Only when the water reaches flood stage: liquidating or liquidated |

Colour means: lake is collateral inside the shielded pool; flood is liquidation. Being healthy is the normal state and
is drawn in ink. A margin call is ink too, because nothing moves.

## Type

- **Barlow:** all UI text. A low-contrast grotesk drawn from road and instrument lettering, legible at a distance like
  the numerals on a staff gauge. Display sizes at 300, UI at 400 and 500. Self-hosted.
- **Barlow Semi Condensed:** gauge numerals and the large readings, so long figures fit a narrow staff.
- **Source Serif 4 italic:** hydrographic labels only: the wordmark, stage names on the staff and the water-line label.
  Topographic maps set water names in italic; that is the reason for the choice.
- **JetBrains Mono:** loan ids, addresses and hashes. Never labels.
- **Figures:** tabular everywhere numbers update or line up.

## Layout

- **Alignment and grid:** left-aligned. The station page is a two-column split: the staff gauge (fixed width) and the
  readings. Below 720 px the staff sits above the readings.
- **Separation:** hairline rules and space. No cards.
- **Tables:** the lender book, the key-turn ledger and the visibility table are real tables. Below 640 px rows stack.
- **Panels:** sheet, 1 px rule, 8 px radius, no shadow, only for the loan id field and the test-price tool.
- **Radius per role:** gauge, hydrograph and rules 0; inputs and buttons 6 px; panels 8 px.
- **Home page:** leads with the flood drill, not a stock hero. A 5:7 split: the claim and the two ways in (the lender
  book, a live station) on the left, the drill on the right; stacked below 960 px. Then, separated by rules: how a loan
  runs (a numbered table, because it is a real sequence), which two keys can do what, what stays private, what the code
  has proven (claim and evidence) and the limits. No logo strip, no icon cards, no statistic row, no closing banner.

## Components

- **Buttons:** ink fill for the one primary action, ink outline for secondary ones. Links are underlined text.
- **Inputs:** sheet, 1 px rule, the unit (USD) inside the field on the right.
- **Status:** a stage sentence, not a pill: "Below action stage", "Action stage: margin call", "Flood stage:
  liquidation", "Closed", "Liquidated". Only "Flood stage: liquidation" and "Liquidated" are coloured.
- **Figures:** a ruled ledger row replaces stat boxes.
- **Key marks:** a filled ink disc means the key acted; a hairline ring means it was not needed.
- **Empty, loading, error:** the staff is drawn without water until the first reading ("No reading yet. The policy
  signer takes one every round."). If the policy signer is unreachable: "The policy signer is not answering at
  <url>. Showing the last report on Base."

## Motion

Two things move, and nothing moves on scroll.

- **The water line** settles at each new reading in 220 ms (`cubic-bezier(0.2, 0, 0, 1)`, transform only).
- **The flood drill**, the home page's one orchestrated moment. It plays once on load and again on "Replay". It starts
  from the live reading of a real loan held by the policy signer, then steps ZEC/USD down in simulated rounds (one
  reading every 160 ms, each moving the water line as above) to the action stage, holds, steps down to the flood stage,
  holds, fills the lender's and the policy signer's key marks (opacity, 220 ms) and draws the split of the collateral
  (scaleX, 280 ms). About twelve seconds in all, then it stays on the final state. Every simulated reading is
  labelled "simulated"; the prices and the split come from the loan's own terms.
- **Reduced motion:** the drill does not play. The same four states (live, action stage, flood stage, split) are shown
  as a table with the same numbers.

Changed on 6 Oct 2026 to add the drill (the earlier version allowed only the water line): a landing page has to show
how a liquidation works before anyone opens a station, and the concept's own instrument can show it in twelve seconds
with real terms. The portfolio rule allows one orchestrated moment; this is it.

## Voice

- **Headlines:** "The collateral never leaves the pool." "Two of three keys move it. Lentic holds one." "Flood stage
  is 75 %."
- **Error message:** "The policy signer is not answering. Showing the last report on Base."
- **Words we use:** station, gauge, water line, stage, round, reading, escrow, key, shielded pool, lender book.
- **Words we never use:** guaranteed, safe, risk-free, yield, earn, trustless, custody (Lentic holds no funds),
  plus the universal list.

## Banned

Universal: section 4 of the portfolio DESIGN.md (gradients, glow, glass, default UI faces, eyebrows, meta strings,
welded arrows, fake sequence numbers, emoji, scroll motion, count-ups, filler copy, unsourced round numbers, em
dashes).

Product-specific:

- Zcash yellow `#F3B724` and cyan `#00B2FF`: they would read as an official Zcash property.
- Base blue `#0000FF`, Colosseum red `#C00000`, Phala acid green `#C5FF2E`: host and sponsor signatures.
- Blue water gradients, waves, ripples, droplets, water textures. Water is one flat tint.
- Padlocks and shields as privacy icons.
- Green, amber and red health pills. Healthy and margin call are ink; only flood is coloured.
- Phone mockups, lifestyle photos and four-box statistic rows (Zodl, Unchained, Firefish).
- A dark theme: a survey sheet is printed; one theme keeps the single colour meaning legible.

## Checks

- `pnpm design:check` runs `scripts/design-check.mjs` on `web` and `README.md`, and runs in CI.
- `pnpm screens` writes 375, 820 and 1280 px screenshots of the home page (drill finished, and with reduced motion),
  the lender book and a station, with a live price and with a test price that puts the water over flood stage, to
  `web/screens/` (not committed).
