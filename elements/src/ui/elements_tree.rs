//! Own the DOM table, reusable editable captions and transient mutation feedback.
use super::*;
use objc2_foundation::NSNumber;
use objc2_quartz_core::{CABasicAnimation, CALayer, CAMediaTiming};
use std::rc::Rc;

#[derive(Clone, Copy)]
pub(super) enum EditMode {
    Hit,
    Name,
    Attribute,
}
#[derive(Default)]
pub struct DelegateIvars {
    pub rows: RefCell<Vec<DomRow>>,
    pub rules: Rc<RefCell<Vec<Rc<StyleRule>>>>,
    pub updating: Cell<bool>,
    pub epoch: Cell<u64>,
    pub width: Cell<f64>,
    pub(super) edit: RefCell<Option<NodeEdit>>,
    pub(super) context: Cell<Option<NodeTarget>>,
}
// SAFETY: Native actions use their declared sender types; callbacks only route
// user events and never retain the application's session or CDP objects.
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[ivars=DelegateIvars]
    pub(super) struct ElementsDelegate;
    unsafe impl NSObjectProtocol for ElementsDelegate {}
    impl ElementsDelegate {
        // Start inline editing at the double-clicked DOM row and attribute.
        #[unsafe(method(editNode:))]
        fn edit(&self, table: &NSTableView) {
            self.begin_edit(table, table.clickedRow(), EditMode::Hit);
        }

        // Select the previous result of the current Elements search.
        #[unsafe(method(previousMatch:))]
        fn previous(&self, _: &AnyObject) {
            crate::app::element_action(Action::Next(true));
        }

        // Select the next result of the current Elements search.
        #[unsafe(method(nextMatch:))]
        fn next(&self, _: &AnyObject) {
            crate::app::element_action(Action::Next(false));
        }

        // Expand or collapse the node identified by its disclosure tag.
        #[unsafe(method(toggleNode:))]
        fn toggle(&self, sender: &NSButton) {
            crate::app::element_action(Action::Toggle(sender.tag() as i64));
        }

        // Present declared actions for the current panel or clicked DOM node.
        #[unsafe(method(elementMenu:))]
        fn menu(&self, sender: &NSButton) {
            self.show_menu(sender);
        }

        // Forward the declared panel-menu tag to its action table.
        #[unsafe(method(elementPanelAction:))]
        fn panel_action(&self, sender: &NSMenuItem) {
            self.panel_command(sender.tag());
        }

        // Forward the context-node menu choice to its validated action owner.
        #[unsafe(method(elementNodeAction:))]
        fn node_action(&self, sender: &NSMenuItem) {
            self.node_command(sender);
        }

        // Forward the pseudo-state choice to the node captured by the menu.
        #[unsafe(method(elementStateAction:))]
        fn state_action(&self, sender: &NSMenuItem) {
            self.state_command(sender.tag());
        }

        // Resolve the declared detail tab to its Elements section.
        #[unsafe(method(elementSection:))]
        fn section(&self, sender: &NSButton) {
            crate::app::element_action(Action::Section(section(sender.tag())));
        }

        // Commit the current Elements draft through the protocol session owner.
        #[unsafe(method(applyMarkup:))]
        fn apply(&self, _: &AnyObject) {
            crate::app::apply_element_edit();
        }

        // Expand or collapse the detail section identified by the sender tag.
        #[unsafe(method(elementDisclosure:))]
        fn disclosure(&self, sender: &NSButton) {
            crate::app::toggle_element_section(sender.tag() as usize);
        }

        // Present matching CSS rules for the current element.
        #[unsafe(method(elementRules:))]
        fn rules(&self, sender: &NSButton) {
            self.show_rules(sender);
        }

        // Select the matching CSS rule identified by its menu tag.
        #[unsafe(method(selectRule:))]
        fn rule(&self, sender: &NSMenuItem) {
            crate::app::select_element_rule(sender.tag() as usize);
        }
    }
    // SAFETY: Text callbacks refer only to the field editor borrowed by AppKit for this row.
    unsafe impl NSControlTextEditingDelegate for ElementsDelegate {
        // Finish the native edit through the dedicated inline editor owner.
        #[unsafe(method(controlTextDidEndEditing:))]
        fn edit_ended(&self, note: &NSNotification) {
            self.finish_edit(note);
        }

        // Delegate native text commands to the inline editor behavior.
        #[unsafe(method(control:textView:doCommandBySelector:))]
        unsafe fn edit_command(
            &self,
            control: &NSControl,
            editor: &NSTextView,
            command: objc2::runtime::Sel,
        ) -> bool {
            self.inline_command(control, editor, command)
        }
    }
    // SAFETY: This declares the text-field superprotocol; editing is handled above.
    unsafe impl NSTextFieldDelegate for ElementsDelegate {}
    unsafe impl NSTableViewDataSource for ElementsDelegate {
        // Report the retained row snapshot that AppKit is about to display.
        #[unsafe(method(numberOfRowsInTableView:))]
        fn count(&self, _: &NSTableView) -> isize {
            self.ivars().rows.borrow().len() as isize
        }
    }
    unsafe impl NSTableViewDelegate for ElementsDelegate {
        // Supply a reusable native cell for the requested visible row.
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn cell(
            &self,
            table: &NSTableView,
            _: Option<&NSTableColumn>,
            row: isize,
        ) -> Option<Retained<NSView>> {
            self.dom_cell(table, row)
        }

        // Supply the native row surface used for selection and mutation feedback.
        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row(&self, _: &NSTableView, _: isize) -> Option<Retained<NSTableRowView>> {
            Some(MutationRow::new().into_super())
        }

        // Route native selection changes to the owning model.
        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selected(&self, note: &NSNotification) {
            self.selected_node(note);
        }
    }
);

// Resolve the declared detail tab to its Elements section.
pub(super) fn section(index: isize) -> Section {
    match index {
        1 => Section::Computed,
        2 => Section::Attributes,
        3 => Section::Html,
        _ => Section::Styles,
    }
}
impl ElementsDelegate {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub(super) fn new() -> Retained<Self> {
        let object = Self::alloc(mtm()).set_ivars(DelegateIvars::default());
        // SAFETY: Initialize this allocated, main-thread NSObject once.
        unsafe { msg_send![super(object), init] }
    }

    // Ignore programmatic selection notifications before routing a user choice.
    fn selected_node(&self, note: &NSNotification) {
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if self.ivars().updating.get() {
            return;
        }
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        let Some(table) = note.object().and_then(|o| o.downcast::<NSTableView>().ok()) else {
            return;
        };
        self.select_row(table.selectedRow());
    }

    // Resolve a visible row to its stable node identity before selecting it.
    fn select_row(&self, row: isize) {
        let id = self
            .ivars()
            .rows
            .borrow()
            .get(row as usize)
            .map(|row| row.id);
        // Resolve the stable identity before using a row index or applying a retained reply.
        if let Some(id) = id {
            crate::app::element_action(Action::Select(id));
        }
    }

    // Reuse the DOM cell and install its owned editing and disclosure targets.
    fn dom_cell(&self, table: &NSTableView, index: isize) -> Option<Retained<NSView>> {
        let rows = self.ivars().rows.borrow();
        let row = rows.get(usize::try_from(index).ok()?)?;
        let cell = dom_cell(table, self);
        cell.update(row);
        Some(cell.into_super().into_super())
    }
}
pub struct MutationIvars {
    flash: Retained<CALayer>,
}
// SAFETY: This native row owns its decorative layer; Core Animation drives
// a single finite opacity animation, with no polling or retained Rust closure.
define_class!(
    #[unsafe(super(NSTableRowView))]
    #[ivars=MutationIvars]
    struct MutationRow;
);
impl MutationRow {
    // Construct the owner and initialize the state used by its subsequent operations.
    fn new() -> Retained<Self> {
        let flash = CALayer::layer();
        flash.setBackgroundColor(Some(&NSColor::systemYellowColor().CGColor()));
        flash.setOpacity(0.);
        let object = Self::alloc(mtm()).set_ivars(MutationIvars { flash });
        // SAFETY: Initialize this allocated row once before attaching its owned layer.
        let row: Retained<Self> = unsafe { msg_send![super(object),initWithFrame:NSRect::ZERO] };
        row.setWantsLayer(true);
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        if let Some(layer) = row.layer() {
            layer.addSublayer(&row.ivars().flash);
        }
        row
    }

    // Animate the mutation feedback only while its transient highlight is active.
    fn pulse(&self) {
        let flash = &self.ivars().flash;
        // SAFETY: This private key stores only the CA opacity animation created below.
        if unsafe { flash.animationForKey(&text("mutation")) }.is_some() {
            return;
        }
        flash.setFrame(self.bounds());
        let animation = CABasicAnimation::animationWithKeyPath(Some(&text("opacity")));
        // SAFETY: Opacity animations accept boxed numeric values, retained by CA.
        unsafe {
            animation.setFromValue(Some(&NSNumber::new_f64(0.25)));
            animation.setToValue(Some(&NSNumber::new_f64(0.)));
        }
        animation.setDuration(0.65);
        flash.addAnimation_forKey(&animation, Some(&text("mutation")));
    }
}

// Mark a visible changed DOM row without rebuilding the entire tree.
pub(super) fn flash_row(table: &NSTableView, row: usize) {
    // Native views and outlets may be absent during reparenting or teardown; use only live objects.
    if let Some(view) = table.rowViewAtRow_makeIfNecessary(row as isize, false)
        && let Some(view) = view.downcast_ref::<MutationRow>()
    {
        view.pulse();
    }
}
pub struct DomTableIvars {
    delegate: Retained<ElementsDelegate>,
}
// SAFETY: This main-thread table retains its delegate and uses native menu/key event signatures.
define_class!(
    #[unsafe(super(NSTableView))]
    #[ivars=DomTableIvars]
    struct DomTable;
    impl DomTable {
        // Re-resolve the plain table fill when AppKit changes theme; semantic material colors must not pick up wallpaper tint.
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            // SAFETY: Preserve NSTableView's native appearance propagation before applying this table's solid background.
            unsafe { let _: () = msg_send![super(self), viewDidChangeEffectiveAppearance]; }
            self.refresh_background();
        }

        // Present declared actions for the current panel or clicked DOM node.
        #[unsafe(method_id(menuForEvent:))]
        fn menu(&self, event: &NSEvent) -> Option<Retained<NSMenu>> {
            self.ivars().delegate.context_menu(self, event)
        }

        // Handle DOM navigation and editing keys before forwarding unrelated events.
        #[unsafe(method(keyDown:))]
        fn key(&self, event: &NSEvent) {
            // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
            if event.keyCode() == 36 || event.keyCode() == 120 {
                // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
                let mode = if event.modifierFlags().contains(NSEventModifierFlags::Shift) {
                    EditMode::Attribute
                } else {
                    EditMode::Name
                };
                self.ivars()
                    .delegate
                    .begin_edit(self, self.selectedRow(), mode);
                return;
            }
            // SAFETY: Unhandled keys retain NSTableView selection and navigation behavior.
            unsafe {
                let _: () = msg_send![super(self),keyDown:event];
            }
        }
    }
);

impl DomTable {
    // Resolve the same component-based text background used by Network, within this view's effective drawing appearance.
    fn refresh_background(&self) {
        let update = block2::StackBlock::new(|| {
            let background = plain_text_background();
            self.setBackgroundColor(&background);
        });
        self.effectiveAppearance()
            .performAsCurrentDrawingAppearance(&update);
    }
}

// Create the DOM table with native scrolling, selection and editing delegates.
pub(super) fn table(delegate: &ElementsDelegate) -> Retained<NSTableView> {
    let object = DomTable::alloc(mtm()).set_ivars(DomTableIvars {
        delegate: delegate.retain(),
    });
    // SAFETY: Initialize this allocated table exactly once with its retained native delegate.
    let table: Retained<DomTable> = unsafe { msg_send![super(object),initWithFrame:NSRect::ZERO] };
    table.setStyle(NSTableViewStyle::Plain);
    table.setRowHeight(23.);
    table.setIntercellSpacing(NSSize::new(0., 0.));
    table.refresh_background();
    table.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::NoColumnAutoresizing);
    let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm()), &text("node"));
    column.setMaxWidth(f64::MAX);
    column.setWidth(500.);
    table.addTableColumn(&column);
    table.setHeaderView(None);
    table.setAccessibilityLabel(Some(&text("DOM tree")));
    // SAFETY: ElementsView owns this dedicated data source/delegate for the table's lifetime.
    unsafe {
        table.setDelegate(Some(ProtocolObject::from_ref(delegate)));
        table.setDataSource(Some(ProtocolObject::from_ref(delegate)));
        table.setTarget(Some(delegate));
        table.setDoubleAction(Some(sel!(editNode:)));
    }
    table.into_super()
}

// Build the short presentation label independently of native action behavior.
pub(super) fn caption(row: &DomRow) -> String {
    let arrow = match (row.expandable, row.expanded) {
        (false, _) => " ",
        (true, false) => "▸",
        (true, true) => "▾",
    };
    format!("{}{} {}", "  ".repeat(row.depth.min(24)), arrow, row.label)
}

// Refresh a materialized row without overwriting an active field editor.
pub(super) fn update_cell(cell: &NSTableCellView, row: &DomRow) {
    // Reused DOM cells own their own indentation and native selection appearance.
    if let Some(cell) = cell.downcast_ref::<DomCell>() {
        cell.update(row);
    }
}

// Measure the same attributed captions AppKit draws, including indentation and Unicode.
pub(super) fn content_width(table: &NSTableView) -> f64 {
    // Native views and outlets may be absent during reparenting or teardown; use only live objects.
    let Some(table) = table.downcast_ref::<DomTable>() else {
        return 0.;
    };
    table.ivars().delegate.ivars().width.get()
}

// Measure the same monospaced captions that AppKit draws in the tree.
pub(super) fn measure_rows(rows: &[DomRow]) -> f64 {
    let font = super::super::controls::code_font(11.);
    rows.iter()
        .map(|row| {
            super::super::controls::attributed(&caption(row), &font, &NSColor::labelColor())
                .size()
                .width
                + 16.
        })
        .fold(0., f64::max)
}

pub struct CellIvars {
    title: Retained<NSTextField>,
    disclosure: Retained<NSButton>,
    leading: std::cell::OnceCell<Retained<NSLayoutConstraint>>,
}

// SAFETY: This native cell changes only its retained caption after AppKit changes selection appearance.
define_class!(
    #[unsafe(super(NSTableCellView))]
    #[ivars=CellIvars]
    pub(super) struct DomCell;
    impl DomCell {
        // Let AppKit propagate selection first, then replace syntax colors with readable selected text.
        #[unsafe(method(setBackgroundStyle:))]
        fn background(&self, style: NSBackgroundStyle) {
            // SAFETY: Preserve NSTableCellView's native background-style propagation.
            unsafe { let _: () = msg_send![super(self), setBackgroundStyle:style]; }
            self.refresh_selection(style);
        }
    }
);

impl DomCell {
    // Recolor materialized content after native selection or focus changes.
    fn refresh_selection(&self, style: NSBackgroundStyle) {
        let title = &self.ivars().title;
        // Selection or focus changes must not overwrite an active draft.
        if title.currentEditor().is_none() {
            set_caption(title, &title.stringValue().to_string());
        }
        self.ivars()
            .disclosure
            .setContentTintColor(Some(&caption_color(style)));
    }

    // Update row content and tree depth without moving the editable text between presentations.
    pub(super) fn update(&self, row: &DomRow) {
        let ivars = self.ivars();
        let title = &ivars.title;
        // Preserve native editing until Enter, Escape or focus loss ends the draft.
        if title.currentEditor().is_none() {
            set_caption(title, &row.label);
        }
        title.setTag(row.id as isize);
        title.setToolTip(Some(&text(
            "Double-click to edit · Shift+Enter adds an attribute · Right-click for styles",
        )));
        let indent = super::super::controls::attributed(
            "  ",
            &super::super::controls::code_font(11.),
            &NSColor::labelColor(),
        )
        .size()
        .width;
        let leading = 4. + row.depth.min(24) as f64 * indent;
        ivars.leading.get().unwrap().setConstant(leading + 18.);
        let button = &ivars.disclosure;
        button.setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(leading, 0.),
            NSSize::new(18., self.bounds().size.height),
        ));
        button.setHidden(!row.expandable);
        button.setTag(row.id as isize);
        button.setTitle(&text(if row.expanded { "▾" } else { "▸" }));
        button.setAccessibilityLabel(Some(&text(if row.expanded {
            "Collapse DOM node"
        } else {
            "Expand DOM node"
        })));
    }
}

// Match native selected text instead of retaining blue/brown syntax over a blue highlight.
fn caption_color(style: NSBackgroundStyle) -> Retained<NSColor> {
    // Emphasized cells sit on the active system selection; inactive cells use native label colors.
    if style == NSBackgroundStyle::Emphasized {
        return NSColor::alternateSelectedControlTextColor();
    }
    NSColor::labelColor()
}

// Render syntax only outside active selection; the native text cell supplies selection state.
pub(super) fn set_caption(field: &NSTextField, source: &str) {
    let style = field
        .cell()
        .map(|cell| cell.backgroundStyle())
        .unwrap_or(NSBackgroundStyle::Normal);
    // Explicit syntax attributes otherwise override AppKit's selected text color.
    if style == NSBackgroundStyle::Emphasized {
        field.setStringValue(&text(source));
        field.setTextColor(Some(&caption_color(style)));
        return;
    }
    field.setAttributedStringValue(&editor::dom_caption(source));
}

// Reuse one DOM-specific native cell with persistent indentation and disclosure ownership.
pub(super) fn dom_cell(table: &NSTableView, delegate: &ElementsDelegate) -> Retained<DomCell> {
    let key = text("DOM.node");
    // SAFETY: This identifier belongs only to DomCell; no nib owner participates in creation.
    if let Some(cell) = unsafe { table.makeViewWithIdentifier_owner(&key, None) }
        .and_then(|view| view.downcast::<DomCell>().ok())
    {
        return cell;
    }
    let title = NSTextField::labelWithString(&text(""), mtm());
    title.setFont(Some(&super::super::controls::code_font(11.)));
    title.setFocusRingType(NSFocusRingType::None);
    title.setDrawsBackground(false);
    title.setMaximumNumberOfLines(1);
    title.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    title.setTranslatesAutoresizingMaskIntoConstraints(false);
    // SAFETY: The widget retains the declared target and delegate for the lifetime of its cells.
    let disclosure = unsafe {
        title.setDelegate(Some(ProtocolObject::from_ref(delegate)));
        NSButton::buttonWithTitle_target_action(
            &text(""),
            Some(delegate),
            Some(sel!(toggleNode:)),
            mtm(),
        )
    };
    disclosure.setBordered(false);
    disclosure.setFont(title.font().as_deref());
    disclosure.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
    let object = DomCell::alloc(mtm()).set_ivars(CellIvars {
        title,
        disclosure,
        leading: std::cell::OnceCell::new(),
    });
    // SAFETY: Initialize this allocated native cell once before installing owned subviews and outlets.
    let cell: Retained<DomCell> = unsafe { msg_send![super(object), initWithFrame:NSRect::ZERO] };
    cell.setIdentifier(Some(&key));
    let ivars = cell.ivars();
    cell.addSubview(&ivars.title);
    cell.addSubview(&ivars.disclosure);
    // The constraint needs the initialized native cell; publish it before exposing the reusable view.
    let leading = ivars
        .title
        .leadingAnchor()
        .constraintEqualToAnchor(&cell.leadingAnchor());
    ivars.leading.set(leading.clone()).unwrap();
    // SAFETY: The cell's subview hierarchy retains this unretained native outlet.
    unsafe {
        cell.setTextField(Some(&ivars.title));
    }
    activate(vec![
        leading,
        ivars
            .title
            .trailingAnchor()
            .constraintEqualToAnchor_constant(&cell.trailingAnchor(), -4.),
        ivars
            .title
            .centerYAnchor()
            .constraintEqualToAnchor(&cell.centerYAnchor()),
    ]);
    cell
}
