# Key tables: provenance

## `keymap.ts` (Map keyboard mode)

Generated from **rustdesk-org/rdev**, https://github.com/rustdesk-org/rdev, commit
`a361d86a8b0245f3618a9efb375c149530a6b599` (the revision rustdesk/rustdesk tag `1.5.0` pins
in its `Cargo.lock`). Licence: MIT, `LICENSE` at that commit:

> MIT License
>
> Copyright (c) 2020 Nicolas Patry
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this
> software and associated documentation files (the "Software"), to deal in the Software
> without restriction, including without limitation the rights to use, copy, modify, merge,
> publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
> to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or
> substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
> INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
> PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
> FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
> OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
> DEALINGS IN THE SOFTWARE.

| Column of `KEY_CODES` | rdev source at that commit                                                                                                                                                 |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| key (DOM `code`)      | `src/keycodes/chrome.rs`: DOM `KeyboardEvent.code` → rdev `Key` (entries with an empty code are left out)                                                                  |
| Windows scancode      | `src/keycodes/windows.rs`, third column (`scancode_from_key`)                                                                                                              |
| Linux X11 keycode     | `src/keycodes/linux.rs` (`code_from_key`)                                                                                                                                  |
| macOS virtual keycode | `src/keycodes/macos.rs` with `src/keycodes/macos_virtual_keycodes.rs`; `NonConvert` and `Convert` as `src/codes_conv.rs` `macos_target_code_from_key` maps them (JIS keys) |

`null` marks a key the platform's rdev table does not have; rdev then converts to nothing
(`conv_keycodes!` in `src/codes_conv.rs`). The peer reads the code by its own platform
(rustdesk 1.5.0 `src/keyboard.rs:1291-1359`, the mobile path through
`usb_hid_code_to_*`, which composes the same `Key` tables). The browser gives `code`, not a
USB HID usage, so the table starts from `chrome.rs` instead of `usb_hid.rs`; both lead to the
same rdev `Key`. The macOS ISO-layout variant (`macos_iso_code_from_key`, which swaps
`Backquote` and `IntlBackslash`) is not included: the controller cannot tell the peer's
physical layout.

To regenerate: check out rdev at the commit above and, for every `(Key, "code")` pair of the
`decl_keycodes!` block in `chrome.rs` with a non-empty code, in order, write
`code: [windows scancode, linux code, macos code]` from the tables above. `keyboard.test.ts`
spot-checks entries against the rdev sources.

## `control-keys.ts` (Legacy and Translate modes)

Not copied from anywhere: DOM `key`/`code` names (UI Events KeyboardEvent key and code
values) mapped onto the protocol's `ControlKey` enum, following the rdev `Key` → `ControlKey`
match of rustdesk/rustdesk tag `1.5.0` `src/keyboard.rs:1059-1150` (AGPL-3.0, as this
console).
