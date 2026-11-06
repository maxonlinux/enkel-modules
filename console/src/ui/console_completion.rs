//! Native completion list; focus stays in the expression field.
use super::*;
use crate::developer::Suggestions;
use objc2::Message;
use std::rc::Rc;
#[derive(Default)]
pub struct CompletionIvars {
    suggestions: RefCell<Option<Rc<Suggestions>>>,
}
// SAFETY: Retained main-thread table data source. All row/index and action ABIs
// match AppKit, and reloadData never calls into the application model.
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[ivars=CompletionIvars]
    pub struct CompletionDelegate;
    unsafe impl NSObjectProtocol for CompletionDelegate {}
    // SAFETY: NSTableViewDelegate requires this superprotocol even for a read-only table; all editing callbacks are optional.
    unsafe impl NSControlTextEditingDelegate for CompletionDelegate {}
    impl CompletionDelegate {
        // Apply the selected completion through its retained Console owner.
        #[unsafe(method(acceptCompletion:))]
        fn accept(&self, _: &AnyObject) {
            crate::app::accept_native_completion();
        }
    }
    unsafe impl NSTableViewDataSource for CompletionDelegate {
        // Report the retained row snapshot that AppKit is about to display.
        #[unsafe(method(numberOfRowsInTableView:))]
        fn count(&self, _: &NSTableView) -> isize {
            self.ivars()
                .suggestions
                .borrow()
                .as_ref()
                .map_or(0, |list| list.names.len() as isize)
        }
    }
    unsafe impl NSTableViewDelegate for CompletionDelegate {
        // Supply a reusable native cell for the requested visible row.
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn cell(
            &self,
            table: &NSTableView,
            _: Option<&NSTableColumn>,
            row: isize,
        ) -> Option<Retained<NSView>> {
            let list = self.ivars().suggestions.borrow();
            let name = list
                .as_ref()
                .and_then(|list| {
                    usize::try_from(row)
                        .ok()
                        .and_then(|row| list.names.get(row))
                })
                .map_or("", String::as_str);
            let (cell, title) = text_cell(table, &text("completion"), 8.);
            title.setStringValue(&text(name));
            title.setFont(Some(&super::controls::code_font(12.)));
            Some(cell.into_super())
        }
    }
);
pub(super) struct CompletionAnchor {
    editor: Retained<NSTextView>,
    leading: Retained<NSLayoutConstraint>,
    bottom: Retained<NSLayoutConstraint>,
    start: Cell<Option<usize>>,
}
impl CompletionAnchor {
    // Place the completion popup relative to the native input caret.
    pub(super) fn position(&self, host: &NSView) {
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some(start) = self.start.get() else {
            return;
        };
        // Reject a caret that no longer fits the native input text.
        if start > self.editor.string().length() {
            return;
        }
        // Popup positioning needs the input to be attached to a native window.
        let Some(window) = self.editor.window() else {
            return;
        };
        // SAFETY: A valid UTF-16 insertion range from the current completion
        // query; the optional actual-range output is deliberately null.
        let screen = unsafe {
            self.editor.firstRectForCharacterRange_actualRange(
                objc2_foundation::NSRange::new(start, 0),
                std::ptr::null_mut(),
            )
        };
        let glyph = host.convertRect_fromView(window.convertRectFromScreen(screen), None);
        // Do not start work after teardown or before this owner is ready.
        let Some(scroll) = self.editor.enclosingScrollView() else {
            return;
        };
        let input = host.convertRect_fromView(scroll.bounds(), Some(&scroll));
        // Match the text glyph to the eight-point padding of a suggestion row.
        let leading = glyph.origin.x - input.origin.x - 8.;
        let bottom = input.origin.y + input.size.height - glyph.origin.y - glyph.size.height - 8.;
        // Ignore subpixel constraint changes to avoid unnecessary popup relayout.
        if (self.leading.constant() - leading).abs() > 0.01 {
            self.leading.setConstant(leading);
        }
        // Ignore subpixel constraint changes to avoid unnecessary popup relayout.
        if (self.bottom.constant() - bottom).abs() > 0.01 {
            self.bottom.setConstant(bottom);
        }
    }
}
pub struct CompletionPopup {
    view: Retained<NSScrollView>,
    table: Retained<NSTableView>,
    height: Retained<NSLayoutConstraint>,
    delegate: Retained<CompletionDelegate>,
    host: Retained<NSView>,
    pub(super) anchor: std::rc::Rc<CompletionAnchor>,
}
impl CompletionPopup {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub fn new(host: &NSView, input: &NSView, editor: &NSTextView) -> Self {
        let object = CompletionDelegate::alloc(mtm()).set_ivars(CompletionIvars::default());
        // SAFETY: Initialize the allocated NSObject subclass once.
        let delegate: Retained<CompletionDelegate> = unsafe { msg_send![super(object), init] };
        let view = NSScrollView::new(mtm());
        view.setHasVerticalScroller(true);
        view.setAutohidesScrollers(true);
        view.setWantsLayer(true);
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(8.);
            layer.setMasksToBounds(true);
        }
        // AppKit owns the scroll border and its appearance changes; only rounded clipping belongs to the layer.
        view.setBorderType(NSBorderType::BezelBorder);
        view.setBackgroundColor(&NSColor::windowBackgroundColor());
        let table = NSTableView::new(mtm());
        table.setStyle(NSTableViewStyle::Plain);
        table.setBackgroundColor(&NSColor::windowBackgroundColor());
        table.setHeaderView(None);
        table.setRowHeight(22.);
        table.setIntercellSpacing(NSSize::new(0., 0.));
        table.setAllowsEmptySelection(false);
        table.setColumnAutoresizingStyle(
            NSTableViewColumnAutoresizingStyle::UniformColumnAutoresizingStyle,
        );
        let column =
            NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm()), &text("completion"));
        column.setWidth(480.);
        column.setResizingMask(NSTableColumnResizingOptions::AutoresizingMask);
        table.addTableColumn(&column);
        table.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        table.setAccessibilityLabel(Some(&text("JavaScript completions")));
        // SAFETY: Data source/delegate outlive the table. The fixed selector
        // accepts the sender table and returns void.
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(&*delegate)));
            table.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            table.setTarget(Some(&*delegate));
            table.setAction(Some(sel!(acceptCompletion:)));
        }
        view.setDocumentView(Some(&table));
        view.setHidden(true);
        view.setTranslatesAutoresizingMaskIntoConstraints(false);
        host.addSubview(&view);
        let height = view.heightAnchor().constraintEqualToConstant(0.);
        let width = view.widthAnchor().constraintEqualToConstant(320.);
        width.setPriority(750.);
        let leading = view
            .leadingAnchor()
            .constraintEqualToAnchor_constant(&input.leadingAnchor(), -8.);
        leading.setPriority(751.);
        let bottom = view
            .bottomAnchor()
            .constraintEqualToAnchor_constant(&input.topAnchor(), -8.);
        activate(vec![
            leading.clone(),
            bottom.clone(),
            view.leadingAnchor()
                .constraintGreaterThanOrEqualToAnchor_constant(&host.leadingAnchor(), 12.),
            view.widthAnchor()
                .constraintGreaterThanOrEqualToConstant(120.),
            view.trailingAnchor()
                .constraintLessThanOrEqualToAnchor_constant(&host.trailingAnchor(), -12.),
            width,
            height.clone(),
        ]);
        Self {
            view,
            table,
            height,
            delegate,
            host: host.retain(),
            anchor: std::rc::Rc::new(CompletionAnchor {
                editor: editor.retain(),
                leading,
                bottom,
                start: Cell::new(None),
            }),
        }
    }

    // Refresh suggestions only for the current completion query.
    pub fn update(&self, suggestions: Option<&Rc<Suggestions>>, input: Option<&(String, usize)>) {
        let suggestions = suggestions.filter(|list| {
            input.is_some_and(|(text, caret)| {
                *text == list.query.expression && *caret == list.query.caret
            })
        });
        // Keep selection when the current suggestion snapshot is unchanged.
        if self.delegate.ivars().suggestions.borrow().as_ref() == suggestions {
            return;
        }
        self.delegate
            .ivars()
            .suggestions
            .replace(suggestions.cloned());
        self.table.reloadData();
        let count = suggestions.map_or(0, |list| list.names.len());
        self.anchor
            .start
            .set(suggestions.map(|list| list.query.start));
        self.height.setConstant(count.min(4) as f64 * 22.);
        self.view.setHidden(count == 0);
        // An empty result closes the popup rather than presenting an empty window.
        if count == 0 {
            return;
        }
        self.host.layoutSubtreeIfNeeded();
        self.anchor.position(&self.host);
        self.host.layoutSubtreeIfNeeded();
        self.view.tile();
        self.table.setFrameSize(NSSize::new(
            self.view.contentSize().width,
            (count as f64 * 22.).max(self.view.contentSize().height),
        ));
        self.table.sizeLastColumnToFit();
        self.view
            .contentView()
            .scrollToPoint(objc2_foundation::NSPoint::ZERO);
        self.view.reflectScrolledClipView(&self.view.contentView());
        self.table
            .selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(0), false);
        self.table.scrollRowToVisible(0);
    }

    // Report presentation state independently of which tools remain open.
    pub fn visible(&self) -> bool {
        !self.view.isHidden()
    }

    // Hide the popup and clear its native selection without editing Console input.
    pub fn dismiss(&self) {
        self.update(None, None);
    }

    // Move selection.
    pub fn move_selection(&self, delta: isize) -> bool {
        // Keep presentation changes separate from the retained tool state and session lifetime.
        if !self.visible() {
            return false;
        }
        let count = self
            .delegate
            .ivars()
            .suggestions
            .borrow()
            .as_ref()
            .map_or(0, |list| list.names.len());
        // An empty suggestion list has no row that keyboard navigation can select.
        if count == 0 {
            return false;
        }
        let selected = (self.table.selectedRow() + delta).clamp(0, count as isize - 1);
        self.table.selectRowIndexes_byExtendingSelection(
            &NSIndexSet::indexSetWithIndex(selected as usize),
            false,
        );
        self.table.scrollRowToVisible(selected);
        true
    }

    // Retain the selected snapshot across native input updates without copying its query or candidate string.
    pub fn selected(&self) -> Option<(Rc<Suggestions>, usize)> {
        let list = self.delegate.ivars().suggestions.borrow();
        let list = list.as_ref()?;
        let index = usize::try_from(self.table.selectedRow()).ok()?;
        list.names.get(index)?;
        Some((list.clone(), index))
    }
}
