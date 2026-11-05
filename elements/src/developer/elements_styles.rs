//! Retain bounded CSS rules for one selected node and apply versioned rule-body edits.
use super::*;
use std::borrow::Cow;

/// Numeric box geometry stays typed from CDP through native drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxModel {
    pub content: [f64; 2],
    pub margin: [f64; 4],
    pub border: [f64; 4],
    pub padding: [f64; 4],
}
#[derive(Clone, Default, PartialEq)]
pub struct StyleRule {
    pub selector: String,
    pub source: String,
    pub body: String,
    pub category: usize,
    sheet: Option<String>,
    range: Option<Value>,
    inline: bool,
    truncated: bool,
}
impl StyleRule {
    // Expose CSS source only when its stylesheet and range support mutation.
    pub fn editable(&self) -> bool {
        !self.truncated && (self.inline || (self.sheet.is_some() && self.range.is_some()))
    }

    // Produce the selected detail source independently of its presentation label.
    pub fn code(&self) -> String {
        let mut code = format!("{} {{\n", self.selector);
        for (index, line) in self.body.lines().enumerate() {
            // Separate declarations while preserving the original absence of an initial blank line.
            if index > 0 {
                code.push('\n');
            }
            code.push_str("  ");
            code.push_str(line);
        }
        code.push_str("\n}");
        code
    }
}
impl Elements {
    // Request dependent CSS details only when the current selection needs them.
    pub fn flush_details(&mut self) {
        // Do not start work after teardown or before this owner is ready.
        if !self.active {
            return;
        }
        // Resolve the live tree identity before reading or mutating its retained state.
        let Some(id) = self
            .selected
            .filter(|id| self.nodes.get(id).is_some_and(|n| n.kind == 1))
        else {
            return;
        };
        // Reject work from an earlier document or selection; protocol IDs may be reused.
        if self.html_dirty
            && !self.pending.values().any(
                |r| matches!(r,Reply::Html(g,node) if *g==self.selection_generation && *node==id),
            )
        {
            self.html_dirty = false;
            self.load_html(id);
        }
        let styles = self.styles_dirty && !self.styles_pending;
        let layout = (self.computed_dirty || styles) && !self.layout_pending;
        // Use the rule and stylesheet that own the declaration range being displayed or edited.
        if styles {
            self.styles_dirty = false;
            self.styles_pending = true;
            self.command(
                "CSS.getMatchedStylesForNode",
                json!({"nodeId":id}),
                Some(Reply::Styles(self.selection_generation, id)),
            );
        }
        // Skip layout-only requests when no dependent geometry was invalidated.
        if !layout {
            return;
        }
        self.computed_dirty = false;
        self.layout_pending = true;
        // The Styles view needs geometry, not the full computed-property payload.
        if self.section == Section::Computed {
            self.command(
                "CSS.getComputedStyleForNode",
                json!({"nodeId":id}),
                Some(Reply::Computed(self.selection_generation, id)),
            );
        }
        self.command(
            "DOM.getBoxModel",
            json!({"nodeId":id}),
            Some(Reply::Box(self.selection_generation, id)),
        );
    }

    // Set styles.
    pub(super) fn set_styles(&mut self, result: &Value) {
        self.styles_pending = false;
        let previous = std::mem::take(&mut self.rules);
        self.add_style("element.style", &result["inlineStyle"], "inline", 0);
        for entry in result["matchedCSSRules"]
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .take(80)
        {
            self.add_rule(entry, false);
        }
        for inherited in result["inherited"]
            .as_array()
            .into_iter()
            .flatten()
            .take(12)
        {
            self.add_style("element.style", &inherited["inlineStyle"], "inline", 2);
            for entry in inherited["matchedCSSRules"]
                .as_array()
                .into_iter()
                .flatten()
                .rev()
                .take(12)
            {
                self.add_rule(entry, true);
            }
        }
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if self.rules == previous {
            self.rules = previous;
            return;
        }
        self.styles_revision += 1;
        self.detail_revision += 1;
    }

    // Add rule.
    fn add_rule(&mut self, entry: &Value, inherited: bool) {
        let rule = &entry["rule"];
        let selector = matched_selector(entry);
        let origin = rule["origin"].as_str().unwrap_or("stylesheet");
        // Inherited rules need a separate category from rules matching the node itself.
        let category = if inherited {
            2
        } else {
            usize::from(origin == "user-agent")
        };
        self.add_style(&selector, &rule["style"], origin, category);
    }

    // Add style.
    fn add_style(&mut self, selector: &str, style: &Value, origin: &str, category: usize) {
        // Bound retained data and per-event work before accepting more content.
        if style.is_null() || self.rules.len() >= 128 {
            return;
        }
        let raw = style_text(style);
        let sheet = style["styleSheetId"].as_str().map(str::to_owned);
        let source = sheet
            .as_ref()
            .and_then(|id| self.sheets.get(id))
            .map_or(origin, String::as_str);
        self.rules.push(Rc::new(StyleRule {
            selector: selector.into(),
            source: format!(
                "{}:{}",
                source,
                style["range"]["startLine"].as_u64().unwrap_or_default() + 1
            ),
            body: format_declarations(&clipped(&raw, 16384)),
            category,
            sheet,
            range: style.get("range").cloned(),
            inline: origin == "inline" && category == 0,
            truncated: raw.len() > 16384,
        }));
    }

    // Validate the selected stylesheet range before queuing its replacement.
    pub fn edit_rule(&mut self, index: usize, revision: u64, code: &str) {
        // Avoid duplicate in-flight work and wait for the reply that already owns this state.
        if revision != self.styles_revision || self.styles_pending {
            self.status = "Styles changed · press Esc to reload before applying".into();
            self.notice = Some(self.status.clone());
            self.detail_revision += 1;
            return;
        }
        // Avoid duplicate in-flight work and wait for the reply that already owns this state.
        if self.pending.values().any(|r| matches!(r, Reply::Edit(_))) {
            return;
        }
        // Only complete editable source may enter this mutation path; previews stay read-only.
        let Some(rule) = self.rules.get(index).filter(|rule| rule.editable()) else {
            return;
        };
        let prefix = format!("{} {{", rule.selector);
        // Use the rule and stylesheet that own the declaration range being displayed or edited.
        let Some(body) = code
            .trim()
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix('}'))
        else {
            self.status = "Keep the rule selector and braces; edit declarations inside".into();
            self.notice = Some(self.status.clone());
            self.detail_revision += 1;
            return;
        };
        // Reject oversized CSS drafts before serializing a protocol mutation.
        if body.len() > 16384 {
            self.status = "CSS rule exceeds the 16 KiB editor limit".into();
            self.notice = Some(self.status.clone());
            self.detail_revision += 1;
            return;
        }
        // Use the rule and stylesheet that own the declaration range being displayed or edited.
        let (method, params) = if rule.inline {
            (
                "DOM.setAttributeValue",
                json!({"nodeId":self.selected,"name":"style","value":body.trim()}),
            )
        } else {
            (
                "CSS.setStyleTexts",
                json!({"edits":[{"styleSheetId":rule.sheet,"range":rule.range,"text":body}]}),
            )
        };
        self.command(method, params, Some(Reply::Edit(self.epoch)));
    }

    // Retain stylesheet ownership needed to distinguish editable and browser rules.
    pub(super) fn sheet_added(&mut self, header: &Value) {
        // Use the rule and stylesheet that own the declaration range being displayed or edited.
        let Some(id) = header["styleSheetId"].as_str() else {
            return;
        };
        // Bound retained data and per-event work before accepting more content.
        if self.sheets.len() >= 256 {
            return;
        }
        let url = header["sourceURL"].as_str().unwrap_or("");
        let name = url
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("inline stylesheet");
        self.sheets.insert(id.into(), clipped(name, 128));
    }

    // Set computed.
    pub(super) fn set_computed(&mut self, result: &Value) {
        // Retain the full computed snapshot only while its dedicated section is selected.
        if self.section != Section::Computed {
            return;
        }
        self.computed = result["computedStyle"]
            .as_array()
            .into_iter()
            .flatten()
            .take(1500)
            .map(|p| {
                (
                    clipped(p["name"].as_str().unwrap_or_default(), 128),
                    clipped(p["value"].as_str().unwrap_or_default(), 1024),
                )
            })
            .collect();
        self.detail_revision += 1;
    }

    // Set box.
    pub(super) fn set_box(&mut self, model: &Value) {
        self.layout_pending = false;
        let read = |index| model["content"][index].as_f64().unwrap_or_default();
        self.box_model = Some(BoxModel {
            content: [
                (read(2) - read(0)).hypot(read(3) - read(1)),
                (read(6) - read(0)).hypot(read(7) - read(1)),
            ],
            margin: quad_insets(&model["margin"], &model["border"]),
            border: quad_insets(&model["border"], &model["padding"]),
            padding: quad_insets(&model["padding"], &model["content"]),
        });
        self.detail_revision += 1;
    }

    // Add property.
    pub fn add_property(&mut self, name: &str, value: &str) {
        // Resolve the stable identity before using a row index or applying a retained reply.
        let Some(id) = self.selected else {
            return;
        };
        // A shortened preview is not a complete edit source and must not overwrite the page.
        if self.nodes.get(&id).is_some_and(|n| n.truncated) {
            self.status =
                "Attribute preview is truncated; edit a stylesheet declaration instead".into();
            return;
        }
        let name = name.trim();
        // Reject empty or oversized CSS property drafts before queuing a mutation.
        if name.is_empty() || name.len() > 128 || value.len() > 4096 {
            return;
        }
        // Add overrides to the inline declaration; do not rewrite entire sheets.
        let current = self
            .nodes
            .get(&id)
            .and_then(|node| node.attributes.iter().find(|(key, _)| key == "style"))
            .map_or("", |(_, value)| value.as_str());
        let source = format!("{current}; {name}: {value};");
        self.command(
            "DOM.setAttributeValue",
            json!({"nodeId":id,"name":"style","value":source}),
            Some(Reply::Edit(self.epoch)),
        );
    }

    // Force state.
    pub fn force_state(&mut self, state: &str) {
        // Resolve the stable identity before using a row index or applying a retained reply.
        let Some(id) = self.selected else {
            return;
        };
        self.clear_forced();
        self.forced = (!state.is_empty()).then_some(id);
        self.command("CSS.forcePseudoState", json!({"nodeId":id,"forcedPseudoClasses":if state.is_empty() {vec![]} else {vec![state]}}), Some(Reply::Edit(self.epoch)));
    }

    // Clear forced.
    pub(super) fn clear_forced(&mut self) {
        // Resolve the stable identity before using a row index or applying a retained reply.
        if let Some(id) = self.forced.take() {
            self.command(
                "CSS.forcePseudoState",
                json!({"nodeId":id,"forcedPseudoClasses":[]}),
                None,
            );
        }
    }

    // Render complete attribute data for the dedicated source section.
    pub fn attributes_text(&self) -> String {
        self.selected
            .and_then(|id| self.nodes.get(&id))
            .map_or_else(String::new, |node| {
                node.attributes
                    .iter()
                    .map(|(k, v)| {
                        format!("{k}=\"{}\"", v.replace('&', "&amp;").replace('"', "&quot;"))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
    }

    // Render the selected node's computed property snapshot.
    pub fn computed_text(&self) -> String {
        let mut output = String::new();
        for (name, value) in &self.computed {
            // Separate properties in one output buffer rather than allocating and joining a string per property.
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(name);
            output.push_str(": ");
            output.push_str(value);
            output.push(';');
        }
        output
    }
}

// CDP reports which selectors matched; hide unrelated branches of large selector lists.
fn matched_selector(entry: &Value) -> String {
    let list = &entry["rule"]["selectorList"];
    let matches = entry["matchingSelectors"]
        .as_array()
        .into_iter()
        .flatten()
        .take(16)
        .filter_map(|index| index.as_u64().and_then(|index| usize::try_from(index).ok()))
        .filter_map(|index| list["selectors"][index]["text"].as_str())
        .collect::<Vec<_>>()
        .join(", ");
    clipped(
        // Use the complete selector text when the protocol supplied no matching-selector indices.
        if matches.is_empty() {
            list["text"].as_str().unwrap_or("rule")
        } else {
            &matches
        },
        200,
    )
}

// Derive readable box-model measurements from protocol quads.
fn quad_insets(outer: &Value, inner: &Value) -> [f64; 4] {
    let read = |quad: &Value, index: usize| quad[index].as_f64().unwrap_or_default();
    [
        read(inner, 1) - read(outer, 1),
        read(outer, 2) - read(inner, 2),
        read(outer, 5) - read(inner, 5),
        read(inner, 0) - read(outer, 0),
    ]
}

// Author CSS normally supplies its source; reconstruct only rules without it.
fn style_text(style: &Value) -> Cow<'_, str> {
    // Borrow original CSS when Chromium supplied it; reconstructed declarations are only a fallback.
    if let Some(source) = style["cssText"].as_str() {
        return Cow::Borrowed(source);
    }
    style["cssProperties"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["implicit"].as_bool() != Some(true))
        .filter(|p| {
            style.get("styleSheetId").is_none()
                || p.get("range").is_some()
                || p.get("text").is_some()
        })
        .take(200)
        .map(declaration)
        .collect::<Vec<_>>()
        .join("\n")
        .into()
}

// Locate one CSS declaration while respecting strings and comments.
fn declaration(value: &Value) -> String {
    // Validate the incoming text before interpreting it as a control value or navigation target.
    if let Some(text) = value["text"].as_str() {
        return clipped(text, 4096);
    }
    format!(
        "{}: {}{};",
        value["name"].as_str().unwrap_or(""),
        value["value"].as_str().unwrap_or(""),
        if value["important"].as_bool() == Some(true) {
            " !important"
        } else {
            ""
        }
    )
}

// Format only declaration boundaries; quoted strings, comments and functions stay intact.
fn format_declarations(source: &str) -> String {
    let mut output = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut depth = 0_u32;
    let mut comment = false;
    let mut previous = '\0';
    let mut property = true;
    let mut chars = source.trim().chars().peekable();
    while let Some(ch) = chars.next() {
        output.push(ch);
        // Preserve comment contents rather than interpreting their punctuation as declarations.
        if comment {
            comment = !(previous == '*' && ch == '/');
            previous = ch;
            continue;
        }
        // An escaped quote does not end the CSS string.
        if escaped {
            escaped = false;
            previous = ch;
            continue;
        }
        // Punctuation inside a quoted CSS value must not create declaration boundaries.
        if let Some(mark) = quote {
            escaped = ch == '\\';
            // Only the matching quote ends the current CSS string.
            if ch == mark {
                quote = None;
            }
            previous = ch;
            continue;
        }
        match ch {
            '/' if chars.peek() == Some(&'*') => comment = true,
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ':' if depth == 0 && property => {
                property = false;
                // Avoid inserting a duplicate line break before existing whitespace.
                if chars.peek().is_some_and(|ch| !ch.is_whitespace()) {
                    output.push(' ');
                }
            }
            ';' if depth == 0 => {
                property = true;
                while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                    chars.next();
                }
                // Do not append another declaration separator at the end of the source.
                if chars.peek().is_some() {
                    output.push('\n');
                }
            }
            _ => {}
        }
        previous = ch;
    }
    output.trim().into()
}

#[cfg(test)]
#[path = "../../tests/elements_styles.rs"]
mod tests;
