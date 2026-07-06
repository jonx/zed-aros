//! Native Intuition menus for gpui's application menu bar.
//!
//! gpui's `Platform::set_menus` hands over a menu tree + the keymap; the mac
//! backend turns that into the NSApp menu bar. The Amiga way is a menu strip
//! *per window* (right mouse button over the title bar), so the platform
//! flattens the tree once into [`MenuState::spec`], applies it to every live
//! window (gadtools `CreateMenus`/`SetMenuStrip` in the C glue), and re-applies
//! it to windows opened later. Picking an item comes back as a MENUPICK event
//! carrying an index into [`MenuState::actions`]; the window dispatches it
//! through [`MenuState::on_action`] (gpui's `on_app_menu_action` callback).

use std::ffi::CString;

use gpui::{Action, Keymap, Menu, MenuItem};

/// One flattened menu entry, with its label/commkey storage. The CStrings
/// stay alive for as long as the spec is stored, so the FFI array built in
/// [`crate::window::ArosWindowInner::apply_menus`] can borrow them.
pub(crate) struct FlatMenuItem {
    /// 0 = menu title, 1 = item, 2 = sub-item.
    pub level: i32,
    /// Index into [`MenuState::actions`]; -1 = separator / not pickable.
    pub id: i32,
    pub label: CString,
    pub commkey: Option<CString>,
    pub disabled: bool,
    pub checked: bool,
}

/// Shared between the platform (which owns menu definition) and every window
/// (which receives the MENUPICK events).
#[derive(Default)]
pub(crate) struct MenuState {
    pub actions: Vec<Box<dyn Action>>,
    pub on_action: Option<Box<dyn FnMut(&dyn Action)>>,
    pub spec: Vec<FlatMenuItem>,
}

/// UTF-8 -> ISO-8859-1 (the system charset), lossy: anything past U+00FF
/// becomes '?'. Same convention as the clipboard bridge.
pub(crate) fn to_latin1(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| if (c as u32) <= 0xFF { c as u32 as u8 } else { b'?' })
        .collect()
}

fn latin1_cstring(s: &str) -> CString {
    let mut bytes = to_latin1(s);
    bytes.retain(|&b| b != 0);
    // A latin-1 byte string with NULs stripped can't fail.
    CString::new(bytes).unwrap_or_default()
}

/// The Amiga command key (right Amiga + single character) for an action:
/// the keymap's display binding (last one wins, like the mac menu bar) when
/// it is a plain `cmd-<char>` chord. Anything with ctrl/alt/fn/shift or a
/// named key has no CommKey representation and shows none.
fn commkey_for(action: &dyn Action, keymap: &Keymap) -> Option<CString> {
    let binding = keymap.bindings_for_action(action).last()?;
    let [keystroke] = binding.keystrokes() else {
        return None;
    };
    let m = keystroke.modifiers;
    if !m.platform || m.control || m.alt || m.function || m.shift {
        return None;
    }
    let mut chars = keystroke.key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let upper = c.to_ascii_uppercase();
    if !upper.is_ascii_graphic() {
        return None;
    }
    CString::new(vec![upper as u8]).ok()
}

impl MenuState {
    /// Flatten gpui's menu tree into the level/id list the C glue consumes,
    /// replacing the previous registry.
    pub fn rebuild(&mut self, menus: Vec<Menu>, keymap: &Keymap) {
        self.actions.clear();
        self.spec.clear();

        for menu in menus {
            let disabled = menu.disabled;
            self.push_title(&menu.name, disabled);
            self.push_items(menu.items, 1, keymap);
        }
    }

    fn push_title(&mut self, name: &str, disabled: bool) {
        self.spec.push(FlatMenuItem {
            level: 0,
            id: -1,
            label: latin1_cstring(name),
            commkey: None,
            disabled,
            checked: false,
        });
    }

    fn push_items(&mut self, items: Vec<MenuItem>, level: i32, keymap: &Keymap) {
        for item in items {
            match item {
                MenuItem::Separator => self.spec.push(FlatMenuItem {
                    level,
                    id: -1,
                    label: CString::default(),
                    commkey: None,
                    disabled: false,
                    checked: false,
                }),
                MenuItem::Action {
                    name,
                    action,
                    checked,
                    disabled,
                    ..
                } => {
                    let id = self.actions.len() as i32;
                    let commkey = commkey_for(action.as_ref(), keymap);
                    self.actions.push(action);
                    self.spec.push(FlatMenuItem {
                        level,
                        id,
                        label: latin1_cstring(&name),
                        commkey,
                        disabled,
                        checked,
                    });
                }
                MenuItem::Submenu(submenu) => {
                    // Intuition menus nest exactly one level (item -> sub);
                    // deeper trees flatten into the sub level.
                    self.spec.push(FlatMenuItem {
                        level,
                        id: -1,
                        label: latin1_cstring(&submenu.name),
                        commkey: None,
                        disabled: submenu.disabled,
                        checked: false,
                    });
                    self.push_items(submenu.items, 2, keymap);
                }
                // No OS-populated menus (Services...) on AROS.
                MenuItem::SystemMenu(_) => {}
            }
        }
    }
}
