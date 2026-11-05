//! Declare Elements menus and dispatch commands to stable node targets.
use super::*;
use tree::EditMode;

impl tree::ElementsDelegate {
    // Show rules.
    pub(super) fn show_rules(&self, anchor: &NSButton) {
        let menu = NSMenu::new(mtm());
        for (index, rule) in self
            .ivars()
            .rules
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.category == anchor.tag() as usize)
        {
            // SAFETY: This fixed menu action accepts an NSMenuItem sender.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm()),
                    &text(&format!("{} · {}", rule.selector, rule.source)),
                    Some(sel!(selectRule:)),
                    &text(""),
                )
            };
            item.setTag(index as isize);
            // SAFETY: The widget owns the target during this synchronous menu.
            unsafe {
                item.setTarget(Some(self));
            }
            menu.addItem(&item);
        }
        menu.popUpMenuPositioningItem_atLocation_inView(
            None,
            objc2_foundation::NSPoint::ZERO,
            Some(anchor),
        );
    }

    // Show menu.
    pub(super) fn show_menu(&self, anchor: &NSButton) {
        let menu = self.commands_menu(PANEL_MENU, sel!(elementPanelAction:));
        menu.popUpMenuPositioningItem_atLocation_inView(
            None,
            objc2_foundation::NSPoint::ZERO,
            Some(anchor),
        );
    }

    // Build a menu from declared action data, including explicit separators.
    fn commands_menu(
        &self,
        entries: &[Option<(&str, Command)>],
        action: objc2::runtime::Sel,
    ) -> Retained<NSMenu> {
        let menu = NSMenu::new(mtm());
        for (index, entry) in entries.iter().enumerate() {
            // Reject a missing or mismatched value before the following operation relies on it.
            let Some((title, _)) = entry else {
                menu.addItem(&NSMenuItem::separatorItem(mtm()));
                continue;
            };
            // SAFETY: Each selector below accepts NSMenuItem; the table/widget owns the target.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm()),
                    &text(title),
                    Some(action),
                    &text(""),
                )
            };
            item.setTag(index as isize);
            // SAFETY: This target remains retained by the owning table during native menu tracking.
            unsafe {
                item.setTarget(Some(self));
            }
            menu.addItem(&item);
        }
        menu
    }

    // Capture the clicked row identity before presenting its available commands.
    pub(super) fn context_menu(
        &self,
        table: &NSTableView,
        event: &NSEvent,
    ) -> Option<Retained<NSMenu>> {
        let point = table.convertPoint_fromView(event.locationInWindow(), None);
        let row = table.rowAtPoint(point);
        let id = self
            .ivars()
            .rows
            .borrow()
            .get(usize::try_from(row).ok()?)?
            .id;
        let target = NodeTarget {
            id,
            epoch: self.ivars().epoch.get(),
        };
        self.ivars().context.set(Some(target));
        table.selectRowIndexes_byExtendingSelection(
            &NSIndexSet::indexSetWithIndex(row as usize),
            false,
        );
        let menu = self.commands_menu(NODE_MENU, sel!(elementNodeAction:));
        // The temporary menu retains its table, never the delegate: no persistent ownership cycle.
        for index in [0, 1] {
            // Apply the optional result only while its owning state is available.
            if let Some(item) = menu.itemAtIndex(index) {
                // SAFETY: NSMenuItem retains the live main-thread table until this menu is released.
                unsafe {
                    item.setRepresentedObject(Some(table));
                }
            }
        }
        let states = self.commands_menu(STATE_MENU, sel!(elementStateAction:));
        let item = NSMenuItem::new(mtm());
        item.setTitle(&text("Force state"));
        item.setSubmenu(Some(&states));
        menu.addItem(&item);
        Some(menu)
    }

    // Dispatch the declared panel action by its stable menu tag.
    pub(super) fn panel_command(&self, tag: isize) {
        // Apply the optional result only while its owning state is available.
        if let Some(Some((_, Command::Perform(action)))) = PANEL_MENU.get(tag as usize) {
            crate::app::element_action(action());
        }
    }

    // Apply the chosen pseudo-state to the node captured when the menu opened.
    pub(super) fn state_command(&self, tag: isize) {
        // Resolve the current target before applying work that could otherwise affect a stale owner.
        if let Some(Some((_, Command::Perform(action)))) = STATE_MENU.get(tag as usize)
            && let Some(target) = self.ivars().context.get()
        {
            crate::app::element_node_action(target, action());
        }
    }

    // Revalidate the context node before starting an edit or issuing its declared action.
    pub(super) fn node_command(&self, item: &NSMenuItem) {
        // Resolve the current target before applying work that could otherwise affect a stale owner.
        let Some(target) = self.ivars().context.get() else {
            return;
        };
        // Resolve the live tree identity before reading or mutating its retained state.
        let Some(Some((_, command))) = NODE_MENU.get(item.tag() as usize) else {
            return;
        };
        // The Add attribute action opens the final insertion slot; other edits select the hit value.
        let mode = if matches!(command, Command::Attribute) {
            EditMode::Attribute
        } else {
            EditMode::Name
        };
        match command {
            Command::Perform(action) => crate::app::element_node_action(target, action()),
            Command::Inline | Command::Attribute => {
                let row = self
                    .ivars()
                    .rows
                    .borrow()
                    .iter()
                    .position(|row| row.id == target.id);
                // Native views and outlets may be absent during reparenting or teardown; use only live objects.
                if let Some(row) = row
                    && let Some(table) = item
                        .representedObject()
                        .and_then(|object| object.downcast::<NSTableView>().ok())
                {
                    self.begin_edit(&table, row as isize, mode);
                }
            }
        }
    }
}

// Labels and their targets are one declaration; tags only index this fixed data.
enum Command {
    Inline,
    Attribute,
    Perform(fn() -> Action),
}
const PANEL_MENU: &[Option<(&str, Command)>] = &[
    Some(("Reload DOM", Command::Perform(|| Action::Refresh))),
    None,
    Some(("Undo DOM edit", Command::Perform(|| Action::Undo))),
    Some(("Redo DOM edit", Command::Perform(|| Action::Redo))),
];
const NODE_MENU: &[Option<(&str, Command)>] = &[
    Some(("Edit inline", Command::Inline)),
    Some(("Add attribute", Command::Attribute)),
    Some((
        "Show styles",
        Command::Perform(|| Action::Section(Section::Styles)),
    )),
    Some((
        "Edit as HTML",
        Command::Perform(|| Action::Section(Section::Html)),
    )),
    Some((
        "Edit attributes",
        Command::Perform(|| Action::Section(Section::Attributes)),
    )),
    None,
    Some(("Duplicate element", Command::Perform(|| Action::Duplicate))),
    Some(("Delete element", Command::Perform(|| Action::Delete))),
    None,
    Some(("Hide element", Command::Perform(|| Action::Hide))),
    Some(("Scroll into view", Command::Perform(|| Action::Scroll))),
    Some(("Focus element", Command::Perform(|| Action::Focus))),
    Some(("Copy outer HTML", Command::Perform(|| Action::CopyHtml))),
];
const STATE_MENU: &[Option<(&str, Command)>] = &[
    Some(("Normal", Command::Perform(|| Action::Pseudo("".into())))),
    Some((
        ":hover",
        Command::Perform(|| Action::Pseudo("hover".into())),
    )),
    Some((
        ":active",
        Command::Perform(|| Action::Pseudo("active".into())),
    )),
    Some((
        ":focus",
        Command::Perform(|| Action::Pseudo("focus".into())),
    )),
    Some((
        ":focus-visible",
        Command::Perform(|| Action::Pseudo("focus-visible".into())),
    )),
];
