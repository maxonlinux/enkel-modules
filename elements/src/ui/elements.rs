//! Assemble the independent Elements widget and apply only dirty DOM rows to native controls.
use super::panels::configure_scroll;
use super::*;
use crate::developer::elements::{
    Action, DomRow, Elements, NodeEdit, NodeTarget, Section, StyleRule,
};
use objc2::Message;
#[path = "elements_details.rs"]
mod details;
#[path = "elements_editor.rs"]
mod editor;
#[path = "elements_menu.rs"]
mod menu;
#[path = "elements_tree.rs"]
mod tree;
use details::Details;
#[path = "elements_layout.rs"]
mod layout;
#[path = "elements_sections.rs"]
mod sections;
use super::controls::{
    boundary, icon_button as button, inspector_menu, set_tab, stack, tab_button, text_button,
};
use tree::ElementsDelegate;

pub struct ElementsView {
    view: Retained<NSView>,
    table: Retained<NSTableView>,
    delegate: Retained<ElementsDelegate>,
    layout: Retained<layout::ElementsLayout>,
    details: Details,
    pick: Retained<NSButton>,
    picking: Cell<bool>,
    matches: Retained<NSTextField>,
    previous: Retained<NSButton>,
    next: Retained<NSButton>,
    find_controls: Retained<NSStackView>,

    indexes: RefCell<HashMap<i64, usize>>,
    selected: Cell<Option<i64>>,
}
impl ElementsView {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub fn new(host: &NSView, pick: &NSButton) -> Self {
        let delegate = ElementsDelegate::new();
        let view = NSView::new(mtm());
        view.setTranslatesAutoresizingMaskIntoConstraints(false);
        host.addSubview(&view);
        activate(vec![
            view.topAnchor()
                .constraintEqualToAnchor_constant(&host.topAnchor(), 74.),
            view.leadingAnchor()
                .constraintEqualToAnchor_constant(&host.leadingAnchor(), 1.),
            view.trailingAnchor()
                .constraintEqualToAnchor_constant(&host.trailingAnchor(), -1.),
            view.bottomAnchor()
                .constraintEqualToAnchor_constant(&host.bottomAnchor(), -8.),
        ]);
        let table = tree::table(&delegate);
        let previous = button(
            &delegate,
            "chevron.up",
            "Previous DOM search result",
            sel!(previousMatch:),
        );
        let next = button(
            &delegate,
            "chevron.down",
            "Next DOM search result",
            sel!(nextMatch:),
        );
        let matches = label("");
        matches.setHidden(true);
        let find_controls = stack(&[&*matches as &NSView, &*previous, &*next], 2.);
        find_controls.setTranslatesAutoresizingMaskIntoConstraints(false);
        host.addSubview(&find_controls);
        activate(vec![
            find_controls
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&host.trailingAnchor(), -12.),
            find_controls
                .centerYAnchor()
                .constraintEqualToAnchor_constant(&host.topAnchor(), 54.),
            find_controls.widthAnchor().constraintEqualToConstant(88.),
        ]);
        let dom_scroll = NSScrollView::new(mtm());
        configure_scroll(&dom_scroll);
        dom_scroll.setHasHorizontalScroller(true);
        dom_scroll.setHasVerticalScroller(true);
        dom_scroll.setDocumentView(Some(&table));
        let details = Details::new(&delegate);
        let split_view =
            layout::ElementsLayout::new(&dom_scroll, &details.view, &details.navigation, &delegate);
        pin_edges(&split_view, &view);
        Self {
            view,
            table,
            delegate,
            layout: split_view,
            details,
            pick: pick.retain(),
            picking: Cell::new(false),
            matches,
            previous,
            next,
            find_controls,

            indexes: RefCell::new(HashMap::new()),
            selected: Cell::new(None),
        }
    }

    // Expose the responder that owns this widget's normal keyboard navigation.
    pub fn focus_view(&self) -> &NSView {
        &self.table
    }

    // Apply edit.
    pub fn apply_edit(&self) -> Option<Action> {
        self.details.apply_edit()
    }

    // Reveal the selected node's editor when compact tabs hide its pane.
    pub fn show_details(&self) {
        self.layout.show_details();
        self.details.editor.window().inspect(|window| {
            window.makeFirstResponder(Some(&*self.details.editor));
        });
    }

    // Toggle section.
    pub fn toggle_section(&self, index: usize) {
        self.details.sections.toggle(index);
    }

    // Select rule.
    pub fn select_rule(&self, index: usize) {
        self.details.select_rule(index);
    }

    // Discard the local editor draft and request fresh source when it was stale.
    pub fn reload_draft(&self) {
        self.details.reload_draft();
    }

    // Mark the current editor contents as the submitted base for later dirty checks.
    pub fn submitted(&self) {
        self.details.submitted();
    }

    // Apply DOM structure, row and detail changes without replacing active native editors.
    pub fn update(&self, model: &mut Elements) {
        self.delegate.ivars().updating.set(true);
        self.delegate.ivars().epoch.set(model.document_epoch());
        let changed = std::mem::take(&mut model.changed);
        // Rebuild row structure only after a structural DOM change.
        if std::mem::take(&mut model.structure) {
            self.update_structure(model);
        }
        self.update_rows(model, &changed);
        self.update_selection(model);
        self.details.update(model);
        self.previous.setEnabled(model.search_count > 1);
        self.next.setEnabled(model.search_count > 1);
        // An idle search has no result position to display; keep its navigation controls compact.
        self.matches.setHidden(model.search_count == 0);
        self.matches.setStringValue(&text(&format!(
            "{}/{}",
            if model.search_count == 0 {
                0
            } else {
                model.search_index + 1
            },
            model.search_count
        )));
        self.layout.reflow();
        self.delegate.ivars().updating.set(false);
        for id in changed {
            // Resolve the stable identity before using a row index or applying a retained reply.
            if let Some(row) = self.indexes.borrow().get(&id) {
                tree::flash_row(&self.table, *row);
            }
        }
        // A momentary NSButton changes its own state during mouse tracking.
        if self.picking.replace(model.inspecting()) == model.inspecting() {
            return;
        }
        // Reflect active page picking in the shared header action.
        let (color, title) = if model.inspecting() {
            (
                NSColor::controlAccentColor(),
                "Stop selecting an element (Esc)",
            )
        } else {
            (
                NSColor::secondaryLabelColor(),
                "Select an element on the page (Esc to cancel)",
            )
        };
        self.pick.setContentTintColor(Some(&color));
        self.pick.setAccessibilityLabel(Some(&text(title)));
        self.pick.setToolTip(Some(&text(title)));
    }

    // Apply the smallest changed DOM row interval and retain reusable native cells.
    fn update_structure(&self, model: &Elements) {
        let next = model.rows();
        let (removed, inserted) = changed_range(&self.delegate.ivars().rows.borrow(), &next);
        self.indexes.replace(
            next.iter()
                .enumerate()
                .map(|(index, row)| (row.id, index))
                .collect(),
        );
        self.delegate.ivars().width.set(tree::measure_rows(&next));
        self.delegate.ivars().rows.replace(next);

        // Preserve stable row views/buttons instead of cancelling in-progress clicks with reloadData.
        if !removed.is_empty() || !inserted.is_empty() {
            self.table.beginUpdates();
            self.table.removeRowsAtIndexes_withAnimation(
                &NSIndexSet::indexSetWithIndexesInRange(objc2_foundation::NSRange::new(
                    removed.start,
                    removed.len(),
                )),
                NSTableViewAnimationOptions::empty(),
            );
            self.table.insertRowsAtIndexes_withAnimation(
                &NSIndexSet::indexSetWithIndexesInRange(objc2_foundation::NSRange::new(
                    inserted.start,
                    inserted.len(),
                )),
                NSTableViewAnimationOptions::empty(),
            );
            self.table.endUpdates();
        }
        let visible = self.table.rowsInRect(self.table.visibleRect());
        let rows = self.delegate.ivars().rows.borrow();
        for index in visible.location
            ..visible
                .location
                .saturating_add(visible.length)
                .min(rows.len())
        {
            self.update_cell(index, &rows[index]);
        }
    }

    // Refresh a materialized row without overwriting an active field editor.
    fn update_cell(&self, index: usize, row: &DomRow) {
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        if let Some(cell) = self
            .table
            .viewAtColumn_row_makeIfNecessary(0, index as isize, false)
            .and_then(|view| view.downcast::<NSTableCellView>().ok())
        {
            tree::update_cell(&cell, row);
        }
    }

    // Refresh only dirty DOM captions and extend the cached scrollable width.
    fn update_rows(&self, model: &Elements, changed: &std::collections::HashSet<i64>) {
        let indexes = self.indexes.borrow();
        let mut rows = self.delegate.ivars().rows.borrow_mut();
        for id in changed {
            // Resolve the stable identity before using a row index or applying a retained reply.
            let Some(index) = indexes.get(id) else {
                continue;
            };
            // Resolve the live tree identity before reading or mutating its retained state.
            let Some(node) = model.nodes.get(id) else {
                continue;
            };
            let row = &mut rows[*index];
            row.label = node.label();
            row.expandable = node.count > 0;
            self.delegate.ivars().width.set(
                self.delegate
                    .ivars()
                    .width
                    .get()
                    .max(tree::measure_rows(std::slice::from_ref(row))),
            );
            self.update_cell(*index, row);
        }
    }

    // Select the stable model identity without replaying native user callbacks.
    fn update_selection(&self, model: &Elements) {
        let row = model
            .selected
            .and_then(|id| self.indexes.borrow().get(&id).copied());
        // Apply the optional result only while its owning state is available.
        if let Some(row) = row {
            // Avoid replaying a native selection that already represents the same model node.
            if self.table.selectedRow() == row as isize && self.selected.get() == model.selected {
                return;
            }
            self.table
                .selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row), false);
            // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
            if self.selected.replace(model.selected) != model.selected {
                self.table.scrollRowToVisible(row as isize);
            }
        } else {
            // SAFETY: AppKit action takes an optional, unused sender.
            unsafe {
                self.table.deselectAll(None);
            }
            self.selected.set(None);
        }
    }
}
impl Drop for ElementsView {
    // Disconnect native callbacks before releasing the objects that retain their targets.
    fn drop(&mut self) {
        // SAFETY: Disconnect unretained callbacks before releasing the retained delegate.
        unsafe {
            self.table.setDelegate(None);
            self.table.setDataSource(None);
        }
        self.view.removeFromSuperview();
        self.find_controls.removeFromSuperview();
    }
}

// Two linear boundary scans retain the common prefix/suffix, without a quadratic tree diff.
fn changed_range(
    old: &[DomRow],
    next: &[DomRow],
) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let prefix = old
        .iter()
        .zip(next)
        .take_while(|(a, b)| a.id == b.id)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(next[prefix..].iter().rev())
        .take_while(|(a, b)| a.id == b.id)
        .count();
    (prefix..old.len() - suffix, prefix..next.len() - suffix)
}
#[cfg(test)]
#[path = "../../tests/elements_tree.rs"]
mod tests;
