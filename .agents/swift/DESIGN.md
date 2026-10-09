# Milky native design direction

Status: implemented native panel with real Rust app, settings, folder, and file
search, plus explicit fixture mode.
Adapted from the user's supplied frontend-design guidance for a native macOS
launcher. These defaults can be refined through real visual and interaction QA.

## Intent

Milky should feel like a quiet, precise native instrument. The user is in the
middle of doing something else: invoke, recognize a result, act, and return to
work. Search and the selected result carry the hierarchy. Chrome stays subordinate.

The identity comes from a generous, uncluttered query field above compact,
carefully aligned results, with an unmistakable but restrained selection state.
Make that composition excellent before adding decorative branding.

Before a substantial visual change, briefly state its purpose, the direction,
constraints, and one or two concrete choices that express it. Follow the existing
direction unless there is a reason to revise it; do not reinvent the app per screen.

## Typography, color, and material

- Use the macOS system typeface deliberately: system-native is the chosen
  direction. Define query, result title, subtitle, and keyboard-hint roles with
  clear size/weight differences. Metadata remains readable rather than faint.
- Use native bundle icons for app and settings results; use SF Symbols for
  interface actions and as an icon fallback. Align icon bounds and text
  baselines. Avoid emoji as UI iconography.
- Centralize typography, spacing, colors, radii, and motion in a small Swift
  theme/token definition. Do not scatter magic values through views.
- Follow system light/dark appearance. Use semantic surface and text roles,
  one functional accent, and a selection treatment distinguishable without color alone.
- A subtle warm light surface or cool dark surface may carry character, but
  legibility and system accessibility preferences take precedence.
- Native material is appropriate when it improves integration with the desktop.
  Test on bright and busy wallpapers and provide an opaque treatment under
  Reduce Transparency. Avoid stacked glass effects, animated gradients, glow,
  oversized shadows, and a rounded card around every result.
- Use spacing first, then subtle dividers or surface changes to separate content.
  Keep corner treatment and elevation consistent.

## Composition

Start with one compact floating panel: query field, result list, and only useful
action hints. As provisional layout values, try roughly 640 pt wide, 48–56 pt
result rows, and up to eight visible results with scrolling. These are starting
values to inspect, not reasons to clip content or exceed the available screen.

Use a small spacing scale, with tighter spacing within a result than between
major regions. Give the query room without making the panel feel like a website
hero. Bound height to the display's visible frame and avoid jumps on each keystroke.
Adapt to small displays, display scaling, longer localized strings, and larger
text where supported. Web/mobile breakpoints do not apply to this native panel.

Titles truncate deliberately. Subtitles disambiguate duplicate names and paths;
preserve useful path endings and make the full value accessible. Do not expose
match scores, ABI details, or debugging controls in the normal search experience.

## Interaction

- Invocation immediately places text focus in search. Choose and document a
  shortcut that can be changed; do not take over the user's Spotlight shortcut
  or change system settings without an explicit request.
- Up/Down moves selection predictably; Enter invokes the selected result;
  Escape dismisses. Preserve normal text editing and input-method composition.
- Mouse interaction and keyboard selection share one state. Pointer movement
  should not unexpectedly undo an intentional keyboard selection.
- On dismissal, return focus to the previous app when appropriate. On action,
  activate the target app and avoid restoring focus over it.
- A running app should offer switching/activation behavior. A closed app should
  open. Show actionable feedback for a failed operation.
- Start with the primary action. Secondary actions and previews arrive when
  real result types need them; do not fill the first panel with placeholders.
- Keep invocation and typing immediate. Motion must not gate input or actions.
  Use brief, subtle transitions only where they explain a change; respect Reduce Motion.
- For a non-exact query with a ranked top result, keep the user's text intact
  and show that destination as a compact Tab-completable suggestion beside the
  field, with its icon at the trailing edge. Tab completes the query; Return
  continues to perform the primary action. This works for abbreviations as well
  as prefixes, and the suggestion stays unavailable while results are stale.

## States and accessibility

Design empty query, no matches, search in progress, engine unavailable, action
failure, long lists, and missing icons. Keep short searches from flashing a spinner.
Do not invent usage suggestions for an empty query when there is no real signal.
Make failures distinguishable from zero results and provide recovery where possible.

Provide VoiceOver names, selected-state information, accessible action labels,
and logical navigation. Aim for at least 4.5:1 contrast for normal text and 3:1
for large text and essential control indicators. Check actual composited material
backgrounds as well as opaque colors. Never use color as the sole status cue.

## Review standard

Inspect the built panel with realistic results, including duplicate `IDLE` names,
long paths, missing icons, and changing results. Refine alignment, density,
truncation, contrast, focus, and timing together. Distinctiveness should emerge
from precise execution of this direction; adding more features is not visual polish.


## Current inspected defaults — 2026-09-11

The fixture panel is 640 × 554 pt (clamped to available screen space), with
54 pt rows and scrolling. An opaque system window surface follows appearance;
material is deferred after focus-state contrast inspection. Panel geometry does
not animate; Reduce Motion disables result transitions. The selected row combines
a restrained accent fill, a leading stripe, and a Return glyph. Paths truncate
in the middle; full names and paths remain in accessibility labels and tooltips.
A persistent Fixtures footer discloses development mode. Empty-query helper
text describes explicit fixture inputs rather than invented usage suggestions.


## Real search mode — 2026-09-13

Normal launch uses an Apps & Settings footer and a neutral empty-query prompt.
Only explicit fixture mode shows the Fixtures label and development examples.
Layout and keyboard interaction are shared; real order, titles, subtitles, and
primary actions come from Rust.
Startup/ABI/response failures remain visible errors with Retry, never demo data.

## Invocation accessibility — 2026-09-14

The query field has a concise keyboard-use description. Selected results,
errors, and a real no-match state request a VoiceOver announcement when
VoiceOver is enabled; selected-result announcements include the parent path to
distinguish duplicates. The persistent selected stripe remains a shape cue when
color differentiation is reduced. Secondary text is tested for 4.5:1 contrast
against normal and selected surfaces in both appearances.

## Result and search-state refinement — 2026-10-08

Result rows show Rust-provided subtitles directly. App subtitles use the
containing folder (for duplicate `IDLE` results, `Python 3.12` and `Python 3.13`);
settings subtitles use their pane name, or `System Settings` for a pane result.
Settings rows use the icon from their System Settings pane bundle, while apps
retain their native application icons. SF Symbols remain the fallback when a
bundle icon is unavailable.
The title is the best matching name for the destination. The full app path stays
available to assistive technology; settings expose their title, pane, and kind.

This supersedes the earlier two-folder location label: core owns destination
identity and supplies a concise subtitle that works consistently for apps and
settings.

When a new query is still within the short-search grace period, the result area
stays visually quiet instead of showing an empty-state icon. Once the delay
passes, the empty area identifies the ongoing search; quick searches do not
flash a loading state. No-match and failure states retain separate symbols,
messages, and retry behavior.

## Query transition refinement — 2026-10-08

Keep the current result rows and selected highlight in place while a changed
query is searching. Disable actions during that interval so a visible result
from the previous query cannot launch by mistake. When new results arrive,
stable destination IDs let SwiftUI animate insertions, removals, and reordering
with a brief fade. If the search lasts past the existing progress delay, gently
dim the old rows while the search indicator is visible. Reduce Motion disables
these transitions. A completed new query still resets selection to its first
result; same-query refresh preserves a selected ID when it remains available.

## Panel candidates — 2026-10-09

Three fixture-only presentations are available for review at the same 640 × 554
pt size. `balanced` retains the current 54 pt rows, inline Tab suggestion, and
icon at the end of the query line. `compact` uses 44 pt rows, a shorter header
and footer, and type labels to make a longer mixed list scannable. `guided`
places the top suggestion on a separate line and uses 58 pt rows with type
labels; it favors hierarchy over visible result count. All preserve the core's
result order and existing keyboard behavior. This is an experiment, not a
chosen product direction; normal Rust mode remains `balanced`.

Use `scripts/run-fixtures.sh --design=balanced|compact|guided`, then type
`mixed`, or render `scripts/snapshot.sh --query mixed --design DESIGN
--appearance light|dark`. The `mixed` fixture intentionally combines an app,
setting, folder, file, and duplicate app names in a fixed display order.

## Chosen panel and file interactions — 2026-10-09

The user chose `hybrid`: balanced 54 pt rows and inline suggestion with compact
type labels. It is now the real-mode default. Keep the unified result order;
type labels clarify mixed rows without grouping or changing ranking. The footer
exposes Actions (Command-K) and, for a file, Inspect (Command-P). An action
menu occupies the result area and uses Up/Down, Return, and Escape. Inspection
occupies the same area with a Quick Look preview and Back control. Escape
returns to the result list before dismissing the launcher. The query remains
visible and editable; changing it closes either overlay immediately.
