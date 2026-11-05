//! Small native request list and a read-only details pane.
use super::controls::{ACTION_ROW_HEIGHT, action_row_separator};
use super::panels::configure_scroll;
use super::*;
use crate::developer::network::{Detail, Network, Summary};
use std::rc::Rc;
#[path = "network_layout.rs"]
mod layout;
#[path = "network_range.rs"]
mod range;
#[path = "network_table.rs"]
mod table;
#[path = "network_timeline.rs"]
mod timeline;
use table::{RequestScroll, request_list, same_cells};
#[derive(Default)]
pub struct NetworkIvars {
    rows: RefCell<Vec<Summary>>,
    updating: Cell<bool>,
    detail: Cell<Detail>,
}
// SAFETY: Main-thread AppKit data source/delegate with retained model snapshots.
// Programmatic reload/selection notifications are suppressed to avoid Ui reentry.
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[ivars=NetworkIvars]
    pub struct NetworkDelegate;
    unsafe impl NSObjectProtocol for NetworkDelegate {}
    // SAFETY: NSTableViewDelegate requires this superprotocol even for a read-only table; all editing callbacks are optional.
    unsafe impl NSControlTextEditingDelegate for NetworkDelegate {}
    impl NetworkDelegate {
        // Open native search on the selected details view with the correct responder.
        #[unsafe(method(findNetwork:))]
        fn find(&self, _: &AnyObject) {
            crate::app::find_network_details();
        }

        // Create the Network detail area with its native tab and text owners.
        #[unsafe(method(loadResponse:))]
        fn body(&self, _: &AnyObject) {
            crate::app::load_native_response_body();
        }

        // Choose the request-detail section associated with the native tab identifier.
        #[unsafe(method(selectDetail:))]
        fn detail(&self, sender: &NSButton) {
            self.ivars()
                .detail
                .set(Detail::from_index(sender.tag()));
            crate::app::refresh_native_network();
        }
    }
    unsafe impl NSTableViewDataSource for NetworkDelegate {
        // Report the retained row snapshot that AppKit is about to display.
        #[unsafe(method(numberOfRowsInTableView:))]
        fn count(&self, _: &NSTableView) -> isize {
            self.ivars().rows.borrow().len() as isize
        }
    }
    unsafe impl NSTableViewDelegate for NetworkDelegate {
        // Reassign the fixed outside edge after AppKit changes the visible column order.
        #[unsafe(method(tableView:didDragTableColumn:))]
        fn moved_column(&self, table: &NSTableView, _: &NSTableColumn) {
            // The table may be detached while its retained columns are being restored.
            if let Some(scroll) = table.enclosingScrollView()
                && let Some(scroll) = scroll.downcast_ref::<RequestScroll>()
            {
                scroll.fit_columns();
            }
        }

        // Supply a reusable native cell for the requested visible row.
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn cell(
            &self,
            table: &NSTableView,
            column: Option<&NSTableColumn>,
            row: isize,
        ) -> Option<Retained<NSView>> {
            self.cell_view(table, column, row)
        }

        // Route native selection changes to the owning model.
        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selected(&self, notification: &NSNotification) {
            // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
            if self.ivars().updating.get() {
                return;
            }
            let key = notification
                .object()
                .and_then(|object| object.downcast::<NSTableView>().ok())
                .and_then(|table| usize::try_from(table.selectedRow()).ok())
                .and_then(|row| self.ivars().rows.borrow().get(row).map(|row| row.key));
            crate::app::select_native_request(key);
        }
    }
);
impl NetworkDelegate {
    // Reuse a request cell and update only the column it owns.
    fn cell_view(
        &self,
        table: &NSTableView,
        column: Option<&NSTableColumn>,
        row: isize,
    ) -> Option<Retained<NSView>> {
        let identifier = column?.identifier();
        let key = identifier.to_string();
        let rows = self.ivars().rows.borrow();
        let row = rows.get(usize::try_from(row).ok()?)?;
        let status;
        let value: &str = match key.as_str() {
            "method" => &row.method,
            "status" => {
                status = row.status.to_string();
                &status
            }
            "kind" => &row.kind,
            "size" => &row.size,
            "time" => &row.time,
            "initiator" => &row.initiator,
            _ => &row.name,
        };
        let (cell, title) = text_cell(table, &identifier, table::CELL_PADDING);
        title.setStringValue(&text(value));
        // Resolve the stable identity before using a row index or applying a retained reply.
        title.setToolTip(Some(&text(if key == "initiator" {
            &row.source
        } else {
            value
        })));
        // Handle failure before exposing a partial result as usable state.
        let color = if key == "status" && row.status.is_error() {
            NSColor::systemRedColor()
        } else {
            NSColor::labelColor()
        };
        title.setTextColor(Some(&color));
        Some(cell.into_super())
    }
}
pub struct NetworkView {
    split: Retained<layout::NetworkLayout>,
    table: Retained<NSTableView>,
    timeline: Retained<timeline::Timeline>,
    details: RequestDetails,
    load: Retained<NSButton>,
    find: Retained<NSButton>,
    summary: Retained<NSTextField>,
    delegate: Retained<NetworkDelegate>,
}
impl NetworkView {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub fn new(host: &NSView) -> Self {
        let object = NetworkDelegate::alloc(mtm()).set_ivars(NetworkIvars::default());
        // SAFETY: Initialize this allocated NSObject exactly once.
        let delegate: Retained<NetworkDelegate> = unsafe { msg_send![super(object), init] };
        let view = NSView::new(mtm());
        view.setClipsToBounds(true);
        view.setTranslatesAutoresizingMaskIntoConstraints(false);
        host.addSubview(&view);
        activate(vec![
            view.leadingAnchor()
                .constraintEqualToAnchor_constant(&host.leadingAnchor(), 1.),
            view.trailingAnchor()
                .constraintEqualToAnchor_constant(&host.trailingAnchor(), -1.),
            view.topAnchor()
                .constraintEqualToAnchor_constant(&host.topAnchor(), 94.),
            view.bottomAnchor()
                .constraintEqualToAnchor(&host.bottomAnchor()),
        ]);
        let table = request_list(&delegate);
        let list_host = NSView::new(mtm());
        list_host.setClipsToBounds(true);
        let timeline = timeline::Timeline::new();
        mount_scroll(&table, &list_host);
        let details = RequestDetails::new(&delegate);
        let split = layout::NetworkLayout::new(&timeline, &list_host, &details.view);

        // The host supplies a profile namespace; packages never read browser profile files.
        let key = format!(
            "Enkel.Network.{}.v7",
            enkel_module_sdk::appkit::profile_key()
        );
        split.autosave(&key);
        table.setAutosaveName(Some(&text(&key)));
        table.setAutosaveTableColumns(true);

        // SAFETY: Fixed action signature matches the retained target above.
        let load = unsafe {
            NSButton::buttonWithTitle_target_action(
                &text("Load response body"),
                Some(&*delegate),
                Some(sel!(loadResponse:)),
                mtm(),
            )
        };
        load.setBezelStyle(NSBezelStyle::Push);
        load.setControlSize(NSControlSize::Small);
        let summary = label("No requests yet — reload the page");
        summary.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        summary.setFont(Some(&NSFont::systemFontOfSize(10.)));
        summary.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let find = super::controls::icon_button(
            &delegate,
            "magnifyingglass",
            "Find in Network details (⌘+F)",
            sel!(findNetwork:),
        );
        find.setEnabled(false);
        let footer = footer(&summary, &load, &find);
        for child in [&*split as &NSView, &*footer] {
            child.setTranslatesAutoresizingMaskIntoConstraints(false);
            view.addSubview(child);
        }
        let separator = action_row_separator(&view, &footer);
        activate(vec![
            split.topAnchor().constraintEqualToAnchor(&view.topAnchor()),
            split
                .leadingAnchor()
                .constraintEqualToAnchor(&view.leadingAnchor()),
            split
                .trailingAnchor()
                .constraintEqualToAnchor(&view.trailingAnchor()),
            split
                .bottomAnchor()
                .constraintEqualToAnchor(&separator.topAnchor()),
            footer
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&view.leadingAnchor(), 12.),
            footer
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&view.trailingAnchor(), -12.),
            footer
                .heightAnchor()
                .constraintEqualToConstant(ACTION_ROW_HEIGHT),
        ]);
        Self {
            split,
            table,
            timeline,
            details,
            load,
            find,
            summary,
            delegate,
        }
    }

    // Expose the responder that owns this widget's normal keyboard navigation.
    pub fn focus_view(&self) -> &NSView {
        &self.table
    }

    // Route the shared Find action to the selected Network detail text.
    pub fn find_in_details(&self) {
        self.details.find();
    }

    // Select the stable model identity without replaying native user callbacks.
    fn update_selection(&self, key: Option<u64>) {
        let index = self
            .delegate
            .ivars()
            .rows
            .borrow()
            .iter()
            .position(|row| Some(row.key) == key)
            .map_or(-1, |index| index as isize);
        // Skip an unchanged native request selection to preserve responder and scroll state.
        if self.table.selectedRow() == index {
            return;
        }
        self.delegate.ivars().updating.set(true);
        // Native selection can be negative when no request row is selected.
        if let Ok(index) = usize::try_from(index) {
            self.table.selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(index),
                false,
            );
            self.table.scrollRowToVisible(index as isize);
        } else {
            // SAFETY: AppKit's standard action accepts an optional sender.
            unsafe {
                self.table.deselectAll(None);
            }
        }
        self.delegate.ivars().updating.set(false);
    }

    // Refresh request rows, timeline and selected details from retained capture data.
    pub fn update(&self, network: &Network, query: &str) {
        let mut rows = network.rows(query);
        self.timeline
            .update(&rows, network.selected(), network.timeline_bounds());
        // Keep the overview intact; move matching summaries into the native table.
        let total = rows.len();
        let interval = self.timeline.interval();
        rows.retain(|row| timeline::overlaps(row.span, interval));
        // Reload request cells only when their displayed fields changed; timing alone updates the chart.
        if !same_cells(&self.delegate.ivars().rows.borrow(), &rows) {
            self.delegate.ivars().updating.set(true);
            self.delegate.ivars().rows.replace(rows);
            self.table.reloadData();
            self.delegate.ivars().updating.set(false);
        }
        self.update_selection(network.selected());
        let mut section = self.delegate.ivars().detail.get();
        // Fall back to a retained detail section when the selected request has no data for this one.
        if !network.detail_available(section) {
            section = Detail::Info;
            self.delegate.ivars().detail.set(section);
        }
        self.details.update(network, section);
        let selected = network.selected().is_some();
        self.split.set_details_visible(selected);
        self.find.setEnabled(selected);
        self.load
            .setHidden(section != Detail::Response || !selected);
        self.load.setEnabled(network.can_load_body());
        let count = self.delegate.ivars().rows.borrow().len();
        self.summary.setStringValue(&text(&format!(
            "{count} of {total} requests · newest 300 retained"
        )));
    }
}

impl Drop for NetworkView {
    // Disconnect native callbacks before releasing the objects that retain their targets.
    fn drop(&mut self) {
        // SAFETY: Disconnect native callbacks before releasing their Rust owner.
        unsafe {
            self.table.setDelegate(None);
            self.table.setDataSource(None);
        }
    }
}

// Attach the request table once; the shared scroll configuration owns its native clip and indicators.
fn mount_scroll(table: &NSTableView, host: &NSView) {
    let object = RequestScroll::alloc(mtm()).set_ivars(());
    // SAFETY: Initialize this allocated scroll view exactly once.
    let scroll: Retained<RequestScroll> =
        unsafe { msg_send![super(object),initWithFrame:host.bounds()] };
    configure_scroll(&scroll);
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(true);
    scroll.setDocumentView(Some(table));
    scroll.setDrawsBackground(true);
    scroll.refresh_background();
    pin_edges(&scroll, host);
}

// Keep summary and request actions outside the scrolling content region.
fn footer(summary: &NSTextField, load: &NSButton, find: &NSButton) -> Retained<NSStackView> {
    let bar = super::controls::stack(&[summary as &NSView, load, find], 8.);
    bar.setDetachesHiddenViews(true);
    summary
        .setContentHuggingPriority_forOrientation(249., NSLayoutConstraintOrientation::Horizontal);
    activate(vec![
        find.widthAnchor().constraintEqualToConstant(28.),
        find.heightAnchor()
            .constraintEqualToConstant(ACTION_ROW_HEIGHT),
    ]);
    bar
}

struct RequestDetails {
    view: Retained<NSView>,
    text: Retained<NSTextView>,
    sections: [Retained<NSButton>; 5],
    selected: Cell<(Option<u64>, Detail)>,
    // Keep one bounded UTF-8 snapshot; unchanged details need no AppKit conversion.
    content: RefCell<Rc<str>>,
}
impl RequestDetails {
    // Open the embedded native find bar on the selected details view.
    fn find(&self) {
        // A finder needs a selected request; the details pane is collapsed otherwise.
        if self.selected.get().0.is_none() {
            return;
        }
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        if let Some(window) = self.text.window() {
            window.makeFirstResponder(Some(&*self.text));
        }
        let sender = NSMenuItem::new(mtm());
        sender.setTag(NSFindPanelAction::ShowFindPanel.0 as isize);
        // SAFETY: AppKit accepts a sender with the standard NSFindPanelAction tag.
        unsafe {
            self.text.performFindPanelAction(Some(&sender));
        }
    }

    // Construct the owner and initialize the state used by its subsequent operations.
    fn new(delegate: &NetworkDelegate) -> Self {
        let (detail_scroll, text_view) = super::panels::text_editor("Request details");
        let view = NSView::new(mtm());
        view.setClipsToBounds(true);
        let sections = detail_sections(delegate);
        let tabs = super::controls::stack(&sections.each_ref().map(|tab| &**tab as &NSView), 0.);
        let width = sections
            .iter()
            .map(|tab| tab.intrinsicContentSize().width)
            .sum();
        tabs.setFrame(NSRect::new(
            objc2_foundation::NSPoint::ZERO,
            NSSize::new(width, 32.),
        ));
        let navigation = super::controls::tab_strip(&tabs);
        super::controls::boundary(&navigation);
        view.addSubview(&navigation);
        view.addSubview(&detail_scroll);
        navigation.setTranslatesAutoresizingMaskIntoConstraints(false);
        detail_scroll.setTranslatesAutoresizingMaskIntoConstraints(false);
        activate(vec![
            navigation
                .topAnchor()
                .constraintEqualToAnchor_constant(&view.topAnchor(), 4.),
            navigation.heightAnchor().constraintEqualToConstant(32.),
            navigation
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&view.leadingAnchor(), 12.),
            navigation
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&view.trailingAnchor(), -12.),
            detail_scroll
                .topAnchor()
                .constraintEqualToAnchor_constant(&navigation.bottomAnchor(), 4.),
            detail_scroll
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&view.leadingAnchor(), 12.),
            detail_scroll
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&view.trailingAnchor(), -12.),
            detail_scroll
                .bottomAnchor()
                .constraintEqualToAnchor(&view.bottomAnchor()),
        ]);
        Self {
            view,
            text: text_view,
            sections,
            selected: Cell::new((None, Detail::Info)),
            content: RefCell::new(Rc::from("")),
        }
    }

    // Refresh selected request text while preserving an unchanged native selection.
    fn update(&self, network: &Network, section: Detail) {
        let detail = network.details(section);
        for (index, detail) in Detail::ALL.iter().enumerate() {
            self.sections[index].setEnabled(network.detail_available(*detail));
            super::controls::set_tab(&self.sections[index], *detail == section);
        }
        // Reveal a newly chosen tab without undoing the user's scroll during unrelated request updates.
        if self.selected.get().1 != section {
            let tab = &self.sections[section as usize];
            tab.scrollRectToVisible(tab.bounds());
        }
        // Preserve text selection and scroll position when the selected detail text is unchanged.
        let changed = {
            let current = self.content.borrow();
            !Rc::ptr_eq(&current, &detail) && current.as_ref() != detail.as_ref()
        };
        if changed {
            let selection = self.text.selectedRange();
            self.text.setString(&text(&detail));
            self.text.didChangeText();
            // Restore selection only for the same request/section and a still-valid text range.
            if self.selected.get() == (network.selected(), section)
                && selection.location.saturating_add(selection.length)
                    <= self.text.string().length()
            {
                self.text.setSelectedRange(selection);
            }
        }
        // Share the current model snapshot even when another request produced identical text.
        self.content.replace(detail);
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if self.selected.replace((network.selected(), section)) != (network.selected(), section) {
            self.text
                .scrollRangeToVisible(objc2_foundation::NSRange::new(0, 0));
        }
    }
}

// Declare the request-detail choices independently of displayed selection state.
fn detail_sections(delegate: &NetworkDelegate) -> [Retained<NSButton>; 5] {
    Detail::ALL.map(|section| {
        let tab = super::controls::tab_button(section.title());
        tab.setTag(section as isize);
        // SAFETY: The owning widget retains this delegate; the fixed action accepts an NSButton sender.
        unsafe {
            tab.setTarget(Some(delegate));
            tab.setAction(Some(sel!(selectDetail:)));
        }
        tab
    })
}
