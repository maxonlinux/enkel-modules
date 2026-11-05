//! Own request columns, opaque table backgrounds and native header layout without replacing AppKit borders.
use super::*;

pub(super) const CELL_PADDING: f64 = 5.;

// SAFETY: Only header-cell text drawing is customized; AppKit owns the header background, borders and mouse tracking.
define_class!(
    #[unsafe(super(NSTableHeaderCell))]
    struct RequestHeaderCell;
    impl RequestHeaderCell {
        // Align titles with row text without replacing drawWithFrame:, which paints native column separators.
        #[unsafe(method(drawInteriorWithFrame:inView:))]
        fn draw_title(&self, mut rect: NSRect, _: &NSView) {
            let title = self.attributedStringValue();
            let height = title.size().height.min(rect.size.height);
            rect.origin.x += CELL_PADDING;
            rect.origin.y += (rect.size.height - height) / 2.;
            rect.size = NSSize::new((rect.size.width - 2. * CELL_PADDING).max(0.), height);
            title.drawInRect(rect);
        }
    }
);
// SAFETY: Forward AppKit's resize callback before fitting the document width.
// AppKit fills the trailing column while preserving interior user-adjustable boundaries.
define_class!(
    #[unsafe(super(NSScrollView))]
    pub(super) struct RequestScroll;
    impl RequestScroll {
        // Re-resolve the solid background when this table enters a different light/dark appearance.
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            // SAFETY: Preserve NSScrollView's appearance propagation before refreshing its document colors.
            unsafe {
                let _: () = msg_send![super(self), viewDidChangeEffectiveAppearance];
            }
            self.refresh_background();
        }

        // Preserve the native resize callback before updating owned layout.
        #[unsafe(method(resizeSubviewsWithOldSize:))]
        fn resized(&self, old: NSSize) {
            // SAFETY: Preserve NSScrollView's inherited resize ABI and behavior.
            unsafe {
                let _: () = msg_send![super(self),resizeSubviewsWithOldSize:old];
            }
            self.fit_columns();
        }
    }
);
impl RequestScroll {
    // Resolve the semantic text background into a plain fill so the table does not acquire a desktop-tinted material.
    pub(super) fn refresh_background(&self) {
        let update = block2::StackBlock::new(|| {
            let background = NSColor::textBackgroundColor()
                .colorUsingType(NSColorType::ComponentBased)
                .unwrap_or_else(NSColor::textBackgroundColor);
            self.setBackgroundColor(&background);
            // The scroll may receive its initial appearance before its document is attached.
            if let Some(table) = self
                .documentView()
                .and_then(|view| view.downcast::<NSTableView>().ok())
            {
                table.setBackgroundColor(&background);
            }
        });
        self.effectiveAppearance()
            .performAsCurrentDrawingAppearance(&update);
    }

    // Adapt optional columns and keep the native outer table edge fixed.
    pub(super) fn fit_columns(&self) {
        // Change fitted columns only when their native geometry or visibility differs meaningfully.
        let Some(table) = self
            .documentView()
            .and_then(|view| view.downcast::<NSTableView>().ok())
        else {
            return;
        };
        let available = self.contentSize().width;
        // Wait for a usable viewport before fitting request columns.
        if available <= 0. {
            return;
        }
        for (id, threshold) in [("size", 330.), ("time", 420.), ("initiator", 600.)] {
            // Change fitted columns only when their native geometry or visibility differs meaningfully.
            if let Some(column) = table.tableColumnWithIdentifier(&text(id))
                && column.isHidden() != (available < threshold)
            {
                column.setHidden(available < threshold);
            }
        }
        let columns = table.tableColumns();
        let last = columns.iter().filter(|column| !column.isHidden()).last();
        for column in &columns {
            // Only boundaries between visible columns accept user resizing; the outside edge stays fixed.
            let mask = if last
                .as_ref()
                .is_some_and(|last| std::ptr::eq(&*column, &**last))
            {
                NSTableColumnResizingOptions::AutoresizingMask
            } else {
                NSTableColumnResizingOptions::AutoresizingMask
                    | NSTableColumnResizingOptions::UserResizingMask
            };
            column.setResizingMask(mask);
        }
        // Sequential native resizing shrinks preceding columns if newly visible ones need more room.
        if (table.frame().size.width - available).abs() >= 0.5 {
            table.setFrameSize(NSSize::new(available, table.frame().size.height));
        }
        table.sizeLastColumnToFit();
    }
}

// Timing updates only repaint the chart; reload cells only for displayed changes.
pub(super) fn same_cells(a: &[Summary], b: &[Summary]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.key == b.key
                && a.name == b.name
                && a.method == b.method
                && a.status == b.status
                && a.kind == b.kind
                && a.size == b.size
                && a.time == b.time
                && a.initiator == b.initiator
                && a.source == b.source
        })
}

// Construct the bounded request table with compact native columns.
pub(super) fn request_list(delegate: &NetworkDelegate) -> Retained<NSTableView> {
    let table = NSTableView::initWithFrame(
        NSTableView::alloc(mtm()),
        NSRect::new(objc2_foundation::NSPoint::ZERO, NSSize::new(500., 0.)),
    );
    table.setStyle(NSTableViewStyle::Plain);
    table.setIntercellSpacing(NSSize::new(0., 0.));
    table.setRowHeight(24.);
    table.setAllowsMultipleSelection(false);
    table.setAccessibilityLabel(Some(&text("Network requests")));
    table.setUsesAlternatingRowBackgroundColors(false);
    // AppKit fills the last column after viewport or user column changes; no custom width redistribution.
    table.setColumnAutoresizingStyle(
        NSTableViewColumnAutoresizingStyle::SequentialColumnAutoresizingStyle,
    );
    table.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    for (id, title, width, minimum) in [
        ("name", "Name", 180., 68.),
        ("method", "Method", 54., 48.),
        ("status", "Status", 50., 45.),
        ("kind", "Type", 54., 45.),
        ("size", "Size", 56., 48.),
        ("time", "Time", 56., 48.),
        ("initiator", "Initiator", 140., 88.),
    ] {
        let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm()), &text(id));
        let object = RequestHeaderCell::alloc(mtm()).set_ivars(());
        // SAFETY: Initialize this header cell once; its column retains the initialized native object.
        let cell: Retained<RequestHeaderCell> =
            unsafe { msg_send![super(object), initTextCell:&*text(title)] };
        column.setHeaderCell(&cell);
        cell.setFont(Some(&NSFont::systemFontOfSize(11.)));
        cell.setTextColor(Some(&NSColor::labelColor()));
        cell.setUsesSingleLineMode(true);
        cell.setWraps(false);
        cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        column.setWidth(width);
        column.setMinWidth(
            (cell.attributedStringValue().size().width + 2. * CELL_PADDING).max(minimum),
        );
        column.setResizingMask(
            NSTableColumnResizingOptions::UserResizingMask
                | NSTableColumnResizingOptions::AutoresizingMask,
        );
        table.addTableColumn(&column);
    }
    // SAFETY: Retained delegate/data source outlive this table.
    unsafe {
        table.setDelegate(Some(ProtocolObject::from_ref(delegate)));
        table.setDataSource(Some(ProtocolObject::from_ref(delegate)));
    }
    let frame = NSRect::new(
        objc2_foundation::NSPoint::ZERO,
        NSSize::new(table.frame().size.width, 20.),
    );
    let header = NSTableHeaderView::initWithFrame(NSTableHeaderView::alloc(mtm()), frame);
    table.setHeaderView(Some(&header));
    table
}
