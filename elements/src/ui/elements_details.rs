//! Present one CSS rule editor and folded rule groups, sharing the editor with markup and computed views.
use super::*;
use editor::CodeEditor;
use sections::Sections;
use std::borrow::Cow;
use std::rc::Rc;

pub(super) struct Details {
    pub view: Retained<NSView>,
    pub navigation: Retained<NSStackView>,
    pub editor: Retained<CodeEditor>,
    style_tab: Retained<NSButton>,
    computed_tab: Retained<NSButton>,
    path: Retained<NSTextField>,
    selector: Retained<NSButton>,
    source: Retained<NSTextField>,
    apply: Retained<NSButton>,
    pub sections: Sections,
    rules: Rc<RefCell<Vec<Rc<StyleRule>>>>,
    chosen: Cell<usize>,
    version: Cell<u64>,
    selected: Cell<Option<i64>>,
    section: Cell<Section>,
    revision: Cell<u64>,
    hint: Retained<NSTextField>,
    editor_height: Retained<NSLayoutConstraint>,
}
impl Details {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub fn new(delegate: &ElementsDelegate) -> Self {
        let view = NSView::new(mtm());
        view.setClipsToBounds(true);
        let style_tab = tab_button("Styles");
        let computed_tab = tab_button("Computed");
        computed_tab.setTag(1);
        for tab in [&style_tab, &computed_tab] {
            // SAFETY: The widget retains its delegate; both actions accept a button sender.
            unsafe {
                tab.setTarget(Some(delegate));
                tab.setAction(Some(sel!(elementSection:)));
            }
            activate(vec![tab.heightAnchor().constraintEqualToConstant(32.)]);
        }
        let more = inspector_menu(delegate);
        activate(vec![
            more.widthAnchor().constraintEqualToConstant(28.),
            more.heightAnchor().constraintEqualToConstant(32.),
        ]);
        let tabs = stack(
            &[&*style_tab, &*computed_tab, &*NSView::new(mtm()), &*more],
            0.,
        );
        boundary(&tabs);
        let path = label("Pick an element on the page");
        path.setFont(Some(&NSFont::systemFontOfSize(11.)));
        path.setLineBreakMode(NSLineBreakMode::ByTruncatingHead);
        let navigation = stack(&[&*tabs, &*path], 6.);
        navigation.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        navigation.setAlignment(NSLayoutAttribute::Leading);
        // Let visible rows determine navigation height; an additional 32/60-point constraint races hidden-row updates.
        navigation.setDetachesHiddenViews(true);
        activate(vec![
            tabs.widthAnchor()
                .constraintEqualToAnchor(&navigation.widthAnchor()),
            path.widthAnchor()
                .constraintEqualToAnchor(&navigation.widthAnchor()),
            tabs.heightAnchor().constraintEqualToConstant(32.),
            path.heightAnchor().constraintEqualToConstant(22.),
        ]);
        let selector = text_button(delegate, "", sel!(elementRules:));
        selector.setAlignment(NSTextAlignment::Left);
        selector.setToolTip(Some(&text("Choose a matched rule or edit inline styles")));
        selector.setFont(Some(&NSFont::boldSystemFontOfSize(12.)));
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        if let Some(cell) = selector.cell() {
            cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        }
        let source = label("");
        source.setFont(Some(&NSFont::systemFontOfSize(10.)));
        source.setTextColor(Some(&NSColor::secondaryLabelColor()));
        source.setLineBreakMode(NSLineBreakMode::ByTruncatingHead);
        let apply = text_button(delegate, "Apply", sel!(applyMarkup:));
        apply.setImage(symbol("checkmark").as_deref());
        apply.setImagePosition(NSCellImagePosition::ImageLeft);
        apply.setEnabled(false);
        let hint = label("Select an element to edit its styles");
        hint.setFont(Some(&NSFont::systemFontOfSize(10.)));
        hint.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let (edit_scroll, editor) =
            CodeEditor::new(&apply, &hint, crate::app::reload_element_draft);
        let sections = Sections::new(delegate);
        let editor_height = edit_scroll.heightAnchor().constraintEqualToConstant(130.);
        editor_height.setPriority(250.);
        let details = Self {
            view,
            navigation,
            editor,
            style_tab,
            computed_tab,
            path,
            selector,
            source,
            apply,
            sections,
            rules: delegate.ivars().rules.clone(),
            chosen: Cell::new(0),
            version: Cell::new(u64::MAX),
            selected: Cell::new(None),
            section: Cell::new(Section::Styles),
            revision: Cell::new(u64::MAX),
            hint: hint.retain(),
            editor_height,
        };
        details.attach(&edit_scroll, &hint);
        details
    }

    // Constrain the selected-rule header, source editor and expandable details.
    fn attach(&self, edit_scroll: &NSView, hint: &NSTextField) {
        let header = stack(&[&*self.selector, &*self.source, &*self.apply], 8.);
        // Navigation stays outside the scrolling document; editor focus and a resized
        // viewport must never scroll Styles/Computed out of the pane.
        let gap = NSView::new(mtm());
        let children = [&*header, edit_scroll, hint, &*gap, &*self.sections.view];
        let body = stack(&children, 8.);
        body.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        body.setAlignment(NSLayoutAttribute::CenterX);
        body.setDetachesHiddenViews(true);
        for child in children {
            activate(vec![
                child
                    .widthAnchor()
                    .constraintEqualToAnchor_constant(&body.widthAnchor(), -24.),
            ]);
        }
        activate(vec![
            gap.heightAnchor()
                .constraintGreaterThanOrEqualToConstant(0.),
            hint.heightAnchor().constraintEqualToConstant(14.),
        ]);
        gap.setContentHuggingPriority_forOrientation(1., NSLayoutConstraintOrientation::Vertical);
        let object = DetailDocument::alloc(mtm()).set_ivars(());
        // SAFETY: Initialize this flipped scroll document once.
        let document: Retained<DetailDocument> = unsafe { msg_send![super(object), init] };
        document.setTranslatesAutoresizingMaskIntoConstraints(false);
        let scroll = NSScrollView::new(mtm());
        configure_scroll(&scroll);
        scroll.setHasVerticalScroller(true);
        scroll.setClipsToBounds(true);
        scroll.setDocumentView(Some(&document));
        let column = stack(&[&*self.navigation, &*scroll], 0.);
        column.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        column.setAlignment(NSLayoutAttribute::CenterX);
        column.setDetachesHiddenViews(true);
        pin_edges(&column, &self.view);
        activate(vec![
            self.navigation
                .widthAnchor()
                .constraintEqualToAnchor_constant(&column.widthAnchor(), -24.),
            scroll
                .widthAnchor()
                .constraintEqualToAnchor(&column.widthAnchor()),
        ]);
        body.setTranslatesAutoresizingMaskIntoConstraints(false);
        document.addSubview(&body);
        let clip = scroll.contentView();
        // Prefer the local viewport height; shrink the editor before allowing document scrolling.
        let fill_height = document
            .heightAnchor()
            .constraintEqualToAnchor(&clip.heightAnchor());
        fill_height.setPriority(750.);
        activate(vec![
            document
                .leadingAnchor()
                .constraintEqualToAnchor(&clip.leadingAnchor()),
            document
                .topAnchor()
                .constraintEqualToAnchor(&clip.topAnchor()),
            document
                .widthAnchor()
                .constraintEqualToAnchor(&clip.widthAnchor()),
            body.topAnchor()
                .constraintEqualToAnchor_constant(&document.topAnchor(), 8.),
            document
                .heightAnchor()
                .constraintGreaterThanOrEqualToAnchor(&clip.heightAnchor()),
            body.leadingAnchor()
                .constraintEqualToAnchor(&document.leadingAnchor()),
            body.trailingAnchor()
                .constraintEqualToAnchor(&document.trailingAnchor()),
            body.bottomAnchor()
                .constraintEqualToAnchor(&document.bottomAnchor()),
            header.heightAnchor().constraintEqualToConstant(22.),
            self.apply.widthAnchor().constraintEqualToConstant(60.),
            self.source
                .widthAnchor()
                .constraintLessThanOrEqualToConstant(130.),
            fill_height,
            edit_scroll
                .heightAnchor()
                .constraintGreaterThanOrEqualToConstant(80.),
            self.editor_height.clone(),
        ]);
    }

    // Refresh selected source while preserving a local draft for the same target.
    pub fn update(&self, model: &Elements) {
        self.hint
            .setToolTip(model.notice.as_deref().map(text).as_deref());
        // Apply the optional result only while its owning state is available.
        if let Some(message) = &model.notice {
            self.hint.setStringValue(&text(message));
        }
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if self.revision.replace(model.detail_revision) == model.detail_revision
            && self.section.get() == model.section
        {
            return;
        }
        let selected = self.selected.replace(model.selected) != model.selected;
        let section = self.section.replace(model.section) != model.section;
        let styles = model.section == Section::Styles;
        self.sections.update(model, styles);
        let rules_changed = self.version.get() != model.styles_revision;
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if styles && !rules_changed && !selected && !section {
            return;
        }
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if rules_changed {
            // Preserve a dirty draft while the same node and section remain selected.
            if !selected && !section && self.editor.dirty() {
                self.editor.stale();
                return;
            }
            self.rules.replace(model.rules.clone());
            self.version.set(model.styles_revision);
            self.chosen.set(
                model
                    .rules
                    .iter()
                    .position(|r| r.category == 0 && !r.body.is_empty())
                    .unwrap_or(0),
            );
        }
        let path = model.breadcrumbs();
        self.path.setHidden(path.is_empty());
        self.path.setStringValue(&text(
            &path[path.len().saturating_sub(3)..]
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join(" › "),
        ));
        set_tab(&self.style_tab, styles);
        set_tab(&self.computed_tab, model.section == Section::Computed);
        // Use the rule and stylesheet that own the declaration range being displayed or edited.
        if styles {
            // Replace source when the draft is clean or the selection moves to another owner.
            if !self.editor.dirty() || selected || section {
                self.load_rule();
            }
            return;
        }
        // Mark incoming source stale instead of overwriting edits for the same target.
        if self.editor.dirty() && !selected && !section {
            return;
        }
        let (title, value, editable): (&str, Cow<'_, str>, bool) = match model.section {
            Section::Computed => ("Computed", model.computed_text().into(), false),
            Section::Attributes => (
                "Attributes",
                model.attributes_text().into(),
                model.markup_editable(),
            ),
            _ => ("HTML", model.html.as_str().into(), model.markup_editable()),
        };
        self.selector.setTitle(&text(title));
        self.source.setStringValue(&text(""));
        self.editor.set_base(&value, editable);
    }

    // Load rule.
    fn load_rule(&self) {
        let rules = self.rules.borrow();
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        let Some(rule) = rules.get(self.chosen.get()) else {
            self.selector.setTitle(&text("No matched styles"));
            self.source.setStringValue(&text(""));
            self.editor.set_base("", false);
            return;
        };
        self.selector.setTitle(&text(&rule.selector));
        self.source.setStringValue(&text(&rule.source));
        self.source.setToolTip(Some(&text(&rule.source)));
        let code = rule.code();
        self.editor_height
            .setConstant((code.lines().count() as f64 * 15. + 20.).clamp(112., 220.));
        self.editor.set_base(&code, rule.editable());
    }

    // Select rule.
    pub fn select_rule(&self, index: usize) {
        // Keep unsaved CSS edits attached to their current rule until explicitly applied or discarded.
        if self.editor.dirty() {
            self.hint
                .setStringValue(&text("Apply or cancel the current draft first"));
            return;
        }
        // Use the rule and stylesheet that own the declaration range being displayed or edited.
        if self.rules.borrow().get(index).is_none() {
            return;
        }
        self.chosen.set(index);
        self.load_rule();
    }

    // Mark the current editor contents as the submitted base for later dirty checks.
    pub fn submitted(&self) {
        self.editor.submitted();
    }

    // Discard the local editor draft and request fresh source when it was stale.
    pub fn reload_draft(&self) {
        let stale = self.editor.is_stale();
        self.editor.cancel();
        // A current draft can restore its base locally; stale source needs a model refresh.
        if !stale {
            return;
        }
        self.revision.set(u64::MAX);
        self.version.set(u64::MAX);
    }

    // Apply edit.
    pub fn apply_edit(&self) -> Option<Action> {
        // Do not submit a draft while its native Apply action is disabled.
        if !self.apply.isEnabled() {
            return None;
        }
        let source = self.editor.string().to_string();
        // Use the rule and stylesheet that own the declaration range being displayed or edited.
        Some(if self.section.get() == Section::Styles {
            Action::Rule(self.chosen.get(), self.version.get(), source)
        } else {
            Action::Markup(source)
        })
    }
}
// SAFETY: A flipped document makes the top of the inspector visible on first open.
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[ivars=()]
    struct DetailDocument;
    unsafe impl NSObjectProtocol for DetailDocument {}
    impl DetailDocument {
        // Use top-down coordinates for content laid out from the leading edge.
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {
            true
        }
    }
);
