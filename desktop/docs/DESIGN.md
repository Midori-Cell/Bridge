# Design rules

Both apps (PC window, Android app) follow Apple's Human Interface Guidelines. The rules below are what was
taken from them, and how they are applied. Sources read: Apple's WWDC25 "Get to know the new design system"
and "Meet Liquid Glass", WWDC19 "What's New in iOS Design" (Dark Mode, semantic colors, base/elevated
backgrounds), WWDC20 "The details of UI typography", and Apple's UI design tips (44 pt controls, 11 pt text).
The HIG pages themselves are scripted pages that could not be read automatically, so topic pages such as Layout
or Sidebars were applied from these talks plus general knowledge of the guidelines, not quoted from the pages.

## Rules and how they show up

| Rule | In the apps |
|---|---|
| Hierarchy comes from layout and grouping, not decoration. No heavy borders or custom bar backgrounds. | Inset grouped lists: rows sit on a lighter rounded surface over the window background, separated by hairlines. No outlined boxes. |
| Sidebar = top-level navigation, selection highlighted. | PC window: sidebar with search, a blue "Pair a device" button that is always visible, the paired phones and PCs (round icon, name, last activity, one-line preview) and "Settings" at the bottom. A request to pair shows as a count and a "Wants to pair" row; the code sheet opens by itself. Each device opens its own chat, with the list of files below it. |
| One tint color, used only for the primary action and selection. "When every element is tinted, nothing stands out." | Blue (the user can choose another highlight color in Settings: purple, pink, orange, green, teal, graphite; each has white text on it at 4.5:1 or more and a lighter variant for text on dark backgrounds) for the one primary button per view and the selected sidebar item. Green/orange/red only for status. |
| Semantic colors that adapt to Light and Dark. Text contrast at least 4.5:1. Dark mode is dimmed, not inverted. | Palette in `gui/src/theme.rs`; text and tint colors were checked by calculation (tint 4.9:1 on white). Follows the system appearance. |
| Base vs elevated backgrounds: layered surfaces are lighter in dark mode. | Window = base, lists and sheets = elevated. |
| Label hierarchy: primary, secondary, tertiary, quaternary. | Titles primary, details secondary, placeholders tertiary, disabled quaternary. |
| Type: left-aligned, hierarchy through a few text styles, never below 11 pt. | Title 24, Body 14, Footnote 12 (captions above groups are 12 and bold). The window uses egui's built-in font; Apple's SF font may not be redistributed, and the built-in font has no bold weight, so "bold" is only slightly heavier. |
| Concentric shapes: inner radius = outer radius - padding. Capsules for prominent large buttons. | Cards 12 px, rows 8 px, primary buttons capsule. |
| Controls at least 44 pt on touch (28-32 px on desktop pointers). Controls sit next to the content they affect. | Desktop buttons 30 px high. Android app will use 48 dp. Actions live in the conversation they belong to. |
| Modality only for tasks that need focus, with a dim layer, visible Cancel and one primary button. Alerts for destructive actions: red destructive button, safe default. | Pairing a phone, pairing a PC, rename, and the "wants to pair" request are sheets (Escape = Cancel, clicking the dim layer does nothing). "Unpair" is an alert with a red button. |
| Clickable things look clickable: the pointer becomes a hand over buttons and list entries. The layout follows the window size (no fixed content width). | `interact_cursor` in `gui/src/theme.rs`, and `on_hover_cursor` on custom rows. Chat, cards and lists fill the window; the chat takes the free height. |
| Actions appear where the content is (hover on desktop, long-press or a bottom sheet on touch). Destructive actions are red and say what they remove. | PC chat: hovering a text bubble shows **Reply / Copy / Delete**; double-clicking a bubble starts a reply. Hovering a file shows a folder button and a red x. Deleting a message removes it on both devices; the file a PC already saved is never touched by the other PC. |
| Incoming things ask first. Progress and its controls live with the conversation, below the chat. | A file from another PC arrives as "NAME wants to send a file" with **Accept** and **Decline** in the Transfers area under the chat (plus a badge in the sidebar and a notification). Running transfers can be cancelled by either side; a cancelled or declined one can be removed from the list on both PCs. |
| Pinch/scroll zoom and one scrolling page, so content is never trapped in a small inner scroll area. | The conversation is one scrolling page (chat, transfers, files); the wheel over the chat only scrolls the chat. The chat height can be dragged (grip under it), otherwise it follows the window up to a limit, then more notes show. Transfers never shrink the chat. Ctrl + mouse wheel zooms 60 to 200 % (also Ctrl +/-) and is remembered. |
| Material (blur/glass) belongs to the navigation layer only, never stacked, never on content. | Sidebar has its own slightly different tone; nothing else is translucent. |
| Status is never color alone. | Dot (filled or hollow) + text ("Online", "Not seen"). Switches show on/off by fill and outline. |
| Clear, short, sentence-case writing. | Verb labels ("Pair", "Send"), plain error messages. |

## Android app

Same rules and the same look as the PC window (mockups K, L, M on the design canvas), with touch sizes:

- **Home:** large title "Bridge", Settings as a round 48 dp button, search field, an always visible blue **Pair a PC**
  button, then the paired PCs as a list (round icon with an online dot, name, last activity, one-line preview).
- **Chat with a PC:** back button, PC name with status, a segmented control **Chat | Files**. The chat has the
  same look as on the PC (bubbles, darker transcript with a border on top and sides), the input box with **Files**,
  **Clipboard** and **Send** (44 to 48 dp) under it.
- **Files** tab: the files only, **Open** / **Show folder** per row and **Clear**.
- Same behavior as on the PC: reply to a message (long-press menu: Reply, Copy, Delete), delete for both devices, an
  offer card "NAME wants to send a file" with Accept / Decline, cancel from both sides, remove a cancelled transfer.
- Pairing scans the QR code of the PC (camera view with a frame); sharing from other apps opens a bottom sheet.
- Targets are at least 48 dp, text at least 13 sp, the single blue tint, and the phone's own light/dark setting.
