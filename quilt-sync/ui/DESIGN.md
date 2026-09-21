---
name: QuiltSync UI Kit
description: The desktop sync app's v2 design system — flat, bordered, two-tier tokens, and a status vocabulary that whispers when nothing is wrong.
colors:
  brand-navy: "#2d306d"
  ink: "#1e1f24"
  ink-muted: "#5b5d65"
  ink-disabled: "#848691"
  ink-on-emphasis: "#ffffff"
  page: "#fafafa"
  surface: "#ffffff"
  surface-muted: "#f3f3f5"
  surface-inset: "#e9eaed"
  hairline: "#e1e1e6"
  edge: "#c5c7cf"
  edge-strong: "#b1b3bf"
  field-edge: "#848691"
  field-edge-hover: "#797b86"
  accent-text: "#545a90"
  accent-emphasis-hover: "#3a3e65"
  accent-muted: "#e9ebf5"
  accent-edge: "#aeb5e4"
  focus-ring: "#545a90"
  overlay-hover: "#0a184212"
  overlay-active: "#030a3722"
  success-mark: "#218358"
  success-ink: "#193b2d"
  success-fill: "#00a32f0b"
  success-edge: "#adddc0"
  neutral-mark: "#5b5d65"
  neutral-ink: "#1e1f24"
  neutral-fill: "#0a184212"
  neutral-edge: "#c5c7cf"
  attention-mark: "#ab6400"
  attention-ink: "#4f3422"
  attention-fill: "#ffde003d"
  attention-edge: "#e9c162"
  danger-mark: "#ce2c31"
  danger-ink: "#641723"
  danger-fill: "#f3000d14"
  danger-edge: "#f4a9aa"
typography:
  title:
    fontFamily: "system-ui, -apple-system, Segoe UI, Helvetica Neue, Arial, sans-serif"
    fontSize: "18px"
    fontWeight: 600
    lineHeight: 1.45
  lead:
    fontFamily: "system-ui, -apple-system, Segoe UI, Helvetica Neue, Arial, sans-serif"
    fontSize: "16px"
    fontWeight: 400
    lineHeight: 1.45
  body:
    fontFamily: "system-ui, -apple-system, Segoe UI, Helvetica Neue, Arial, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.45
  control:
    fontFamily: "system-ui, -apple-system, Segoe UI, Helvetica Neue, Arial, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.25
  label:
    fontFamily: "system-ui, -apple-system, Segoe UI, Helvetica Neue, Arial, sans-serif"
    fontSize: "14px"
    fontWeight: 600
    lineHeight: 1.45
    letterSpacing: "0.07em"
  mono:
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.45
rounded:
  base: "4px"
spacing:
  "1": "4px"
  "2": "8px"
  "3": "12px"
  "4": "16px"
  "5": "20px"
  "6": "24px"
  "7": "28px"
  "8": "32px"
  "10": "40px"
  "12": "48px"
components:
  button-default:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.control}"
    rounded: "{rounded.base}"
    padding: "0 12px"
    height: "32px"
  button-default-hover:
    backgroundColor: "{colors.overlay-hover}"
    textColor: "{colors.ink}"
  button-default-disabled:
    backgroundColor: "{colors.overlay-hover}"
    textColor: "{colors.ink-disabled}"
  button-primary:
    backgroundColor: "{colors.brand-navy}"
    textColor: "{colors.ink-on-emphasis}"
    typography: "{typography.control}"
    rounded: "{rounded.base}"
    padding: "0 12px"
    height: "32px"
  button-primary-hover:
    backgroundColor: "{colors.accent-emphasis-hover}"
    textColor: "{colors.ink-on-emphasis}"
  button-large:
    typography: "{typography.lead}"
    padding: "0 16px"
    height: "40px"
  input-text:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.control}"
    rounded: "{rounded.base}"
    padding: "0 8px"
    height: "32px"
    width: "100%"
  select:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.control}"
    rounded: "{rounded.base}"
    padding: "0 10px"
    height: "32px"
  card:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.base}"
    padding: "12px"
  card-title:
    textColor: "{colors.ink-muted}"
    typography: "{typography.label}"
  state-label-success:
    backgroundColor: "{colors.success-fill}"
    textColor: "{colors.success-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.base}"
    padding: "0 8px 0 6px"
  state-label-neutral:
    backgroundColor: "{colors.neutral-fill}"
    textColor: "{colors.neutral-ink}"
  state-label-attention:
    backgroundColor: "{colors.attention-fill}"
    textColor: "{colors.attention-ink}"
  state-label-danger:
    backgroundColor: "{colors.danger-fill}"
    textColor: "{colors.danger-ink}"
  banner-warning:
    backgroundColor: "{colors.attention-fill}"
    textColor: "{colors.attention-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.base}"
    padding: "8px 12px"
  segmented-control:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink-muted}"
    typography: "{typography.control}"
    rounded: "{rounded.base}"
    padding: "2px"
    height: "32px"
  package-row:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.body}"
    padding: "5px 2px"
    width: "100%"
  package-row-hover:
    backgroundColor: "{colors.overlay-hover}"
    textColor: "{colors.ink}"
  appbar:
    backgroundColor: "{colors.brand-navy}"
    textColor: "{colors.ink-on-emphasis}"
    padding: "0 16px"
    height: "48px"
---

<!-- rumdl-disable MD025 MD036 -->
<!-- MD025: the `title` typography role in the frontmatter trips rumdl's
     frontmatter-title detection. MD036: the bold North Star line is
     required by the DESIGN.md format spec. -->

# Design System: QuiltSync UI Kit

## Overview

**Creative North Star: "The Quiet Instrument"**

QuiltSync watches data packages on a scientist's disk and keeps them level with
a remote catalog. Almost always, nothing is wrong. Forty of forty-three rows say
Latest. A design that celebrates that is a design that shouts forty times a
session and is therefore ignored the one time it matters. So this system is
built as an instrument panel rather than a dashboard: the resting state is
nearly silent, and every visual expenditure is reserved for the row that has
something to report.

The surface language is flat and bordered. There is one shadow in the whole
system and it is for overlays only; depth comes from a ladder of opaque surfaces
and a vocabulary of hairlines. There is one corner radius. There is no webfont,
because the desktop already chose a face and this is a desktop application
rather than a web page. Colour is spent on four status tones and one brand navy,
and the brand navy appears exactly once per screen, under the appbar.

The kit is twenty-eight modules of hand-written Leptos components with no
component library beneath them. Colour comes from Radix scales, vendored; the
token names follow Primer's grammar. Nothing is imported at runtime. That
combination was chosen so
the system could be reasoned about entirely from this repository, and it means
every value below is a decision somebody made and can defend, not a framework
default.

**Key Characteristics:**

- Flat, bordered, and unornamented. One shadow, one radius, no gradients.
- A two-tier token layer: vendored Radix scales underneath, semantic roles on
  top, and component CSS may read only the top tier.
- Four status tones, each carrying colour and a distinct glyph silhouette, so
  the vocabulary survives greyscale.
- Loudness is inversely proportional to frequency: the common state whispers.
- 14px is the floor for every piece of text; hierarchy is carried by weight and
  colour.
- Two themes from one set of rules, because a theme swap re-points tier two and
  touches no component.

## Colors

A near-neutral navy-tinted grey for everything structural, one brand navy, and
three stock status hues that are deliberately not brand-tinted.

### Primary

- **Brand Navy** (`#2d306d`): The appbar's ground, and in the light theme the
  fill of every primary button, checked box, and countdown ring. It is a literal
  rather than a scale step, and it is the same literal in both themes: a brand
  colour that changes hue when the user switches to dark is not a brand colour.
  This is the one surface in the system that is not a step on the page's ladder.
- **Quiet Navy** (`#545a90`): Accent text and the light theme's focus ring. The
  scale's text step rather than its darkest, because the darkest is within a
  hair of the brand navy and a link beside a primary button would be
  indistinguishable from it.

### Neutral

- **Graphite** (`#1e1f24`): Default text, and the only ink for anything a user
  must read closely.
- **Slate** (`#5b5d65`): Muted text. Timestamps, captions, field hints, card
  titles.
- **Paper** (`#ffffff`): Every raised surface in the light theme. Literal white,
  not the grey scale's first step, because that step is a tinted near-white
  darker than the page behind it, which would make cards recede instead of rise.
- **Bench** (`#fafafa`): The page ground.
- **Hairline** (`#e1e1e6`): Row dividers and the rules inside a card.
- **Edge** (`#c5c7cf`): The outline of buttons and cards, whose own content
  identifies them. Its hover partner is one step stronger at `#b1b3bf`.
- **Field Edge** (`#848691`): The outline of anything a user types into or picks
  from, with a hover partner one step further at `#797b86`. An empty field is
  identified by its edge alone, so this role clears 3:1 where the button edge
  does not: 3.3 to 3.6 in light and 3.1 to 3.7 in dark, measured on the page, a
  card and a muted surface.
- **Chalk** (`#848691`): Disabled controls only. Never for text a user has to
  read.

### Status

Four tones, each with a mark colour, an ink, an alpha fill, and an edge. The
mark and the ink are different on purpose: the scale's text step passes AA
against the lightest surfaces but not against the tone's own fill, so text on a
fill uses the darkest step and only the glyph uses the brighter one.

- **Moss** (mark `#218358`, ink `#193b2d`): Success. The resting state.
- **Slate** (mark `#5b5d65`, ink `#1e1f24`): Neutral. A tone that reports a
  number and takes no position, as in "2 files changed".
- **Ochre** (mark `#ab6400`, ink `#4f3422`): Attention. Something needs a
  decision.
- **Rust** (mark `#ce2c31`, ink `#641723`): Danger. Something failed.

### Named Rules

**The Two Levels Rule.** Text has exactly two levels, default and muted, and
there is no token for a third. A third level was measured at 3.38:1 and fails
AA. When something must recede further, spend weight or size, never a lighter
grey.

**The Loudness Tracks Frequency Rule.** Success is not weighted like the other
three tones. It is the state of ninety-three percent of rows, and a tone that
appears on almost every row is not reporting anything by appearing. It gets the
faintest fill in the system and a softer edge than its siblings. Read as log
levels, danger is error, attention is warn, and success is *info*: it says you
are safe, not well done.

**The Whose Move Rule.** Attention and Danger are not degrees of severity. They
say *who the next move belongs to*: Attention is waiting on the reader, Danger
is something the surface cannot fix by itself. So a package with uncommitted
changes is Attention and one with conflicts is Danger, although both are one
operation from resolution and the second is not obviously worse. Neutral sits
outside that pair — it reports a fact and takes no position on it — and Success
is the resting state. Picking a tone by how bad it feels is how the four collapse
into two.

**The Unbranded Status Rule.** The greys and the accent blue are custom scales
tinted toward the brand navy. Green, amber, and red are stock and must stay
stock. A green pulled toward navy stops reading as good, and reading as good is
the only job that green has.

**The Field Edge Rule.** Anything a user types into or picks from takes the field
edge, and buttons and cards take the lighter one. The split exists because an
empty field is identified by its outline alone, so that outline is held to the
3:1 floor for non-text contrast, while a button's own label identifies it.

**The Composite Fill Rule.** Status fills are alpha, never solid. A row tints on
hover, so a fill's backdrop is not knowable in advance; a solid fill sits on a
hovered row like a sticker that missed the hover, while an alpha fill
composites. On an untinted surface the alpha step resolves to exactly the solid
step, so nothing was lost to buy this.

## Typography

**Body Font:** the platform's own UI face (`system-ui`, falling back through
`-apple-system`, Segoe UI, Helvetica Neue, Arial)

**Mono Font:** the platform's own mono face (`ui-monospace`, falling back
through SFMono-Regular, Menlo, Consolas)

**Character:** Whatever the desktop has chosen, including a light face. This is
a desktop application and it should look like one, so there is no webfont, no
font loading, and no swap flash. The same page renders in Noto Sans under one
Linux stack and Ubuntu Sans under another, and both are correct.

### Hierarchy

- **Title** (600, 18px, 1.45): Page and section titles.
- **Lead** (400, 16px, 1.45): Card titles and emphasised copy.
- **Body** (400, 14px, 1.45): Everything else. Rows, buttons, pills, captions,
  labels.
- **Control** (400, 14px, 1.25): Single-line text inside a control, where the
  body line height would inflate the box.
- **Label** (600, 14px, 1.45, tracked 0.07em, upper-cased in CSS): Card titles
  that name a block without competing with the rows inside it.

### Named Rules

**The 14px Floor Rule.** Fourteen pixels is the smallest text in the system and
there is deliberately no token beneath it, so nobody can reach for one.
Hierarchy is carried by weight and colour instead of by shrinking.

**The Never Type The Caps Rule.** Upper-casing is a `text-transform`, never a
typed string. Typing a label in capitals makes some screen readers spell it out
as an initialism.

## Layout

The page is a fixed frame rather than a scrolling document. The frame owns the
window height, the appbar and the status regions take what they need, and the
list at the bottom takes the remainder and scrolls inside itself. The chrome
therefore never leaves the screen no matter how long the roster is.

Space is a 4px grid, from 4px up to 48px, named by multiples of the unit.
Control heights sit on that same grid at 24px, 32px, and 40px, which is what
lets a row of mixed controls resolve to one clean line. Horizontal padding
inside a control is free to sit off-grid, because it is optical; height is not.

The page column is capped at 1200px and centred. The appbar is full-bleed while
its contents are not, so the bar reaches both window edges and the logo still
lines up with the regions beneath it. The gap between regions is 24px and is set
in exactly one place.

### Named Rules

**The Regions Carry No Margin Rule.** A region never spaces itself. The page
column owns the rhythm between regions, because a region that spaced itself
would look right alone and wrong beside its neighbours, and there would be no
single place to change it.

**The Honest Width Rule.** A component is reviewed at the width the page
actually gives it. Rows are checked at both the full column and roughly half of
it, and a row that only works at page width is broken, not narrow.

## Elevation & Depth

This system is flat. There are no resting shadows on any component at any size:
not on cards, not on buttons, not on rows. Depth is carried by an opaque surface
ladder and by borders.

The ladder has three rungs above the page ground, and the rule that orders them
is that a raised surface is lighter than what it sits on. In the dark theme the
whole ladder shifts up one step, because the dark page is nearly black and the
first grey step would vanish against it. Separation is deliberately stronger in
dark than in light, since dark surfaces read flatter.

Interaction depth is an alpha overlay rather than a solid tint, for the same
reason status fills are alpha: a solid hover tint only works against the one
background it was picked for, and it disappears the moment the control sits on a
card of that colour. Hover and active are two steps apart on the alpha scale,
because one step apart is invisible.

### Shadow Vocabulary

- **Overlay** (`box-shadow: 0 8px 24px rgb(0 0 0 / 14%)`): The only shadow in
  the system. For things that float above the page and nothing else.

### Named Rules

**The Flat And Bordered Rule.** If a surface needs to separate from its
neighbour, give it a border or move it a rung on the surface ladder. Never reach
for a shadow.

## Shapes

One radius, 4px, everywhere. Almost square with the corners taken off. The
system previously had four radii with no recorded reason why a card was 8px and
a button 6px, which is the signature of a scale nobody chose.

The single legitimate exception is a shape nested against a rounded corner,
where the inner radius must be the outer one minus the inset or the inner curve
bulges against the outer. Write that subtraction literally, as
`calc(var(--q-radius) - 2px)`, so the relationship stays visible. A second token
there would only be a magic number that happens to agree today.

Genuine circles are not radii and are left alone: a spinner is round because it
rotates.

Borders are 1px throughout, and the five border roles are not interchangeable.
The faintest separates rows inside a card. The next outlines buttons and cards,
with a stronger partner for their hover. The last two are the field roles, which
sit darker still, because a field has no content of its own to identify it.

### Named Rules

**The One Radius Rule.** Every corner in the system is 4px, or is derived from
4px by subtracting its own inset. There is no second radius token and adding one
requires retiring this rule first.

## Components

### Buttons

- **Shape:** 4px corners, 1px border, 32px tall (40px at the large size).
- **Default:** White surface, graphite label, edge-coloured border. Padding is
  12px horizontal, 16px at the large size.
- **Primary:** Brand navy fill and border with white ink in the light theme. In
  dark the fill becomes the accent's vivid step instead, because a navy fill on
  a near-black card has no edge.
- **Hover / Focus:** Colour only, over 120ms. Hover darkens the fill and
  strengthens the border. Focus is the global ring.
- **Active:** A 1px diagonal translate, untransitioned, plus a border that
  switches to the page's default ink. That is darker in light and lighter in
  dark, which is the point: the primary variant carries its press entirely in
  geometry and border, because the light theme's accent cannot go darker than
  its resting state.
- **Loading:** A spinning ring replaces the leading icon in the same slot, so an
  iconed button neither reflows nor swaps its layout. The two match at the large
  size; at the default size the ring is 2px narrower, so the width is steady
  rather than identical. Under reduced motion the ring stops and dims rather
  than disappearing, so a frozen ring reads as inactive rather than as a
  rendering bug.

### Cards

- **Corner Style:** 4px.
- **Background:** White in light, the second grey step in dark. Never elevated.
- **Border:** 1px in the control-outline role.
- **Internal Padding:** 12px — or none, for a card whose children are rows that
  carry their own padding. A list that scrolls under sticky headings and ends in
  a full-width footer needs its children to reach the border; everything else
  keeps the padding.
- **Title:** Upper-cased, tracked, muted, at the body size. Letter-spacing does
  the work that shrinking would otherwise do, since the type floor forbids
  shrinking.
- **Rhythm:** The card draws a hairline between any two children, so a card
  holding a mix of row types is divided consistently.

### Inputs / Fields

- **Style:** 32px tall, 4px corners, a 1px field edge, white surface, 8px
  horizontal padding. The field edge and not the button edge: an empty field has
  no content of its own, so its outline is the only thing saying it is there.
- **Hover:** The field edge steps one further from the surface.
- **Focus:** The global ring on the control.
- **Invalid:** The border alone changes, never a fill. A red wash behind text
  the user is still typing makes it harder to read at the moment they are trying
  to fix it, and the border plus the message below already say it twice.
- **Disabled:** Chalk text on a faint overlay, with a not-allowed cursor.

### Select

A native `<select>` stretched invisibly over a bordered wrapper that draws its
own closed state: prefix, value, caret. The native element stays focusable,
labelled, and in the accessibility tree at zero opacity, so the OS still owns
the dropdown. The closed state is drawn to match the text field exactly, because
a select and an input side by side must not disagree about height or border.

### Segmented Control

Native radio inputs, positioned off-screen rather than hidden, with the labels
drawn as segments inside a 32px shell carrying the field edge, because it is a
control you pick from rather than a button you press. The selected segment's
radius is the shell's radius minus the shell's 2px padding. A truncating segment
label is the
signal that the options are too long for a segmented control and belong in a
select.

Two or three short options, and the count is a rule of thumb about width rather
than about taste. It loses to discoverability: the installed package's file
facets are four, counted, and 58% of their toolbar, and they stay segments
because they are the page's statement of *what can be filtered* — a reader
learns that ignoring exists by seeing `Ignored 3`, and behind a select it is
hidden again. Reach for a select when the options are a choice the reader
already knows they have.

An option can be present and unchoosable, for a facet that currently matches
nothing. It is a disabled radio, so the platform takes it out of the tab order
and announces it; it keeps its place, because a segment that disappears at zero
teaches nothing and moves the segments beside it.

### State Label

The state vocabulary made visual **in the list**. An inline chip carrying a 12px
glyph and a short phrase, with four tones. Each tone sets four private custom
properties and the layout paints from them, so no rule in the component names a
colour and a high-contrast or colourblind-safe variant is a tier-two change
alone.

The queue draws the same vocabulary as prose rather than in a chip, so every
label this component holds is a list row's — and the sites word a state
differently on purpose, which is why the vocabulary's own function takes a site
as well as a state.

- **Never truncates and never shrinks.** Every state in the vocabulary is short
  by design, and half a state is worse than a state that pushes the row:
  "conflicts in 2 files" clipped to "conflicts in…" has lost the number that
  made it worth showing. An overflowing chip means the words are wrong, not the
  box.
- **Provisional:** While the page is still resolving a row's true state, the
  chip takes a dashed border and the whole row dims. Static, never animated.

### Package Row

A full-width anchor: namespace, timestamp, state, and nothing clickable inside
it. The namespace truncates from the right; the time column has a 104px floor
that holds the widest relative phrase and is never a cap. The state sits hard
right and is deliberately not a fixed column, so the forty identical chips form
a pattern and the three that differ break it.

Hover is a tint and nothing else. There is no hover underline to go with it,
because the tint already says the row responds and two signals for one target is
noise.

### Queue Row

The row is the link. Every state that names an operation names a page that
performs it, so a queue row has exactly one destination and the whole row goes
there; the verb rides along as text at the right edge rather than as a button,
because a button promises the operation happens on press and none of them do.
One tab stop per row, where the old button was one. Verbs hug their labels, so
the right edges line up and the left ones follow the verb's length. A state that
names no operation has nowhere to send anyone: its row is not a link, carries no
verb, and takes no tab stop at all — the paused row's detail text is read rather
than tabbed to.

The state reads as a **clause after the name** — `org/dataset-c` then `has
conflicts in 2 files` — rather than as a chip beside it. The name keeps default
ink at weight 600 and the clause is muted, which is the treatment the host row
already uses above its sub-line. The words are the vocabulary's, chosen at the
queue site; the list keeps its chips and its noun phrases. When the row narrows
the clause gives way first — it absorbs all but a hundredth of the deficit — and
the name holds its full width until the clause is nearly gone: a name cut
mid-owner is unrecognisable, while a clause cut at its end is still readable up
to the cut.

The tone the chip used to carry becomes a **rule on the row's edge**, inset by the
row's own padding so a column of rows shows separate marks rather than one
unbroken band. Colour and nothing else, which is allowed here because the colour
carries nothing on its own: the clause states the row's state in words, so two
rows of different severity read differently with the colour taken away. The chip
needed a glyph beside it because its words were a short label doing the same job
as its tint; these words are the whole account.

The leading column — the one a cause row fills with its expander, which is what
keeps a cause and a package aligned on their text — stays open and stays empty.
A row that has a state is marked twice already, by its edge rule and by its
clause; a third marker there would say nothing the other two do not. Only a row
that is a bare name, one of the packages an expanded cause speaks for, carries
a bullet.

A row may carry a **detail line** beneath its first, for a state whose account of
itself is longer than a label. Today one state uses it: a paused sync, whose
reason comes from the engine rather than from this vocabulary. That text is shown
verbatim with its line breaks kept, since a workflow rejection is a sentence
followed by one indented line per broken rule. It is indented to the namespace
rather than the bullet, muted, and capped at the same measure as the kit's other
prose. It is allowed to be tall: the queue is where attention is meant to go.

### Banner

A page-level outcome bar reading the same four tone properties the state label
does, so a warning on the page and a warning on a row cannot disagree about what
amber means. Its glyph is 16px against a row chip's 12px, because it is an
announcement rather than an annotation. It animates in by sliding 4px and fading
over 160ms, and has no exit animation. Under reduced motion it simply appears.

### Appbar

Full-bleed brand navy, 48px tall, with a 22px wordmark and its actions pushed to
the right. It has no bottom rule: a grey line over the brand ground reads as a
seam, and the colour change is already the separation.

Buttons on the bar lose their frame and take the bar's ink, because a raised
white pill is right on a page-coloured surface and reads as a white blob on this
one. The bar overrides the focus ring to white, since both themes' accent rings
fail against the brand ground at 1.9:1 and 2.3:1 while white measures 11.9:1.

### Named Rules

**The Never Move Geometry Rule.** Transitions animate colour only. A control
that moves under the cursor is a control you miss. The pressed state's 1px
translate is exempt because it is instantaneous and carries no transition.

**The Two Channels Rule.** Every meaningful distinction is carried twice. A tone
is a colour and a glyph silhouette, so it survives desaturation. A provisional
row is a dashed edge and a dim, so it is findable by someone who is not already
looking for it. The test is whether the channel carries meaning on its own: a
queue row's edge rule is colour and nothing else, and that is allowed, because
the clause beside it states the row's state in words. Take the colour away and
the row still says everything it said.

**The Platform Owns The Keyboard Rule.** Where a native element exists, wrap it
rather than rebuild it. Native select, native radios, native checkbox. This is
why the system ships no listbox, no combobox, and no popover, and why roving
focus and arrow-key movement were never hand-written.

## Do's and Don'ts

### Do

- **Do** read semantic tokens only. Component CSS may name a tier-two role and
  must never name a Radix step or a raw literal.
- **Do** put a status colour's text on the tone's darkest step when it sits on
  the tone's own fill, and use the brighter mark step only for glyphs and for
  bare text on an untinted surface.
- **Do** derive a nested radius as `calc(var(--q-radius) - <inset>)` so the
  relationship is visible in the code.
- **Do** give anything a user types into or picks from the field edge, and keep
  the lighter button edge for buttons and cards.
- **Do** give every control a height from the control scale (24px, 32px, 40px),
  so a row of mixed controls lands on one line.
- **Do** let the page column own the gaps between regions.
- **Do** carry meaning in two channels whenever the distinction matters, so it
  survives greyscale.
- **Do** check both themes before calling a colour change done. The two themes'
  hover directions are opposite: the light accent's hover step is lighter, the
  dark accent's is darker.

### Don't

- **Don't** add a third text colour. Two levels, then weight and size.
- **Don't** add a second radius token.
- **Don't** add a resting shadow to anything. The overlay shadow is for
  overlays.
- **Don't** use a solid fill where the surface beneath can be tinted by hover.
  Reach for the alpha step.
- **Don't** outline a form field with the button edge. An empty field is
  identified by its edge alone and has to clear 3:1.
- **Don't** brand-tint a status hue.
- **Don't** hand-write a hover colour. Go through the token, or it will be wrong
  in one theme.
- **Don't** introduce a type token below 14px.
- **Don't** link a webfont. The desktop's own face is the correct one, including
  a light one.
- **Don't** truncate a state label. If it overflows, the words are wrong.
- **Don't** build a listbox, combobox, or popover. Wrap the native element.
