//! Apply bounded DOM edits to their original node and reject stale drafts without replacing children.
use super::*;

#[derive(Clone, Copy)]
pub struct NodeTarget {
    pub id: i64,
    pub epoch: u64,
}
// A draft snapshots only the edited node's own fields. Children may mutate while
// editing, but changing an attribute on the page must not be overwritten silently.
enum Before {
    Tag {
        name: String,
        attributes: Vec<(String, String)>,
    },
    Value(String),
}
pub struct NodeEdit {
    pub target: NodeTarget,
    before: Before,
    source: String,
    pub selection: std::ops::Range<usize>,
}
impl NodeEdit {
    // Expose the complete editable source rather than the shortened preview.
    pub fn source(&self) -> &str {
        &self.source
    }

    // Move between fields of the current tag draft without committing it.
    pub fn navigate(
        &self,
        source: &mut String,
        selection: std::ops::Range<usize>,
        previous: bool,
    ) -> Option<std::ops::Range<usize>> {
        // Attribute-field navigation is available only for complete opening-tag drafts.
        if !matches!(self.before, Before::Tag { .. }) {
            return None;
        }
        tag::navigate(source, selection, previous)
    }

    // Reject stale drafts when the page has changed the fields being edited.
    fn unchanged(&self, node: &Node) -> bool {
        match &self.before {
            Before::Tag { name, attributes } => {
                node.kind == 1 && node.name == *name && node.attributes == *attributes
            }
            Before::Value(value) => matches!(node.kind, 3 | 8) && node.value == *value,
        }
    }
}
impl Elements {
    // Expose the document generation used to reject reused node IDs.
    pub fn document_epoch(&self) -> u64 {
        self.epoch
    }

    // Revalidate a context-menu target before mutating the selected document.
    pub fn perform_on(&mut self, target: NodeTarget, action: Action) {
        // Reject work from an earlier document or selection; protocol IDs may be reused.
        if target.epoch != self.epoch || !self.nodes.contains_key(&target.id) {
            return;
        }
        self.select(target.id);
        self.perform(action);
    }

    // Snapshot the live node and map the click into its complete editable source.
    pub fn prepare_edit(&self, target: NodeTarget, offset: usize) -> Option<NodeEdit> {
        // Reject work from an earlier document or selection; protocol IDs may be reused.
        if !self.active || target.epoch != self.epoch {
            return None;
        }
        let node = self.nodes.get(&target.id).filter(|node| !node.truncated)?;
        let (before, mut source, selection) = match node.kind {
            1 => {
                let source = tag::source(&node.name, &node.attributes, false);
                let selection = tag::hit_selection(&node.label(), &source, offset)?;
                (
                    Before::Tag {
                        name: node.name.clone(),
                        attributes: node.attributes.clone(),
                    },
                    source,
                    selection,
                )
            }
            3 | 8 => (
                Before::Value(node.value.clone()),
                node.value.clone(),
                0..node.value.encode_utf16().count(),
            ),
            _ => return None,
        };
        // Keep the caret and selection on valid native UTF-16/source boundaries.
        let selection = if matches!(before, Before::Tag { .. })
            && selection.is_empty()
            && selection.start >= source.encode_utf16().count() - 1
        {
            tag::insertion(&mut source)?
        } else {
            selection
        };
        Some(NodeEdit {
            target,
            before,
            source,
            selection,
        })
    }

    // Validate the original target before committing the complete draft.
    pub(super) fn apply_node_edit(&mut self, edit: NodeEdit, source: &str) {
        // Reject work from an earlier document or selection; protocol IDs may be reused.
        if !self.active || edit.target.epoch != self.epoch {
            return;
        }
        // Resolve the current target before applying work that could otherwise affect a stale owner.
        let Some(node) = self.nodes.get(&edit.target.id) else {
            self.edit_notice("The edited node was removed; the draft was not applied");
            return;
        };
        // A shortened preview is not a complete edit source and must not overwrite the page.
        if node.truncated || !edit.unchanged(node) {
            self.edit_notice(
                "This element changed on the page; reopen its editor to use the current attributes",
            );
            return;
        }
        // Bound the mutation before serializing editable source into a protocol command.
        if source.len() > 65536 {
            self.edit_notice("Inline edit exceeds 64 KiB");
            return;
        }
        // An unchanged draft must not create a page mutation or undo entry.
        if source == edit.source() {
            return;
        }
        let id = edit.target.id;
        match edit.before {
            Before::Value(_) => self.command(
                "DOM.setNodeValue",
                json!({"nodeId":id,"value":source}),
                Some(Reply::Edit(self.epoch)),
            ),
            Before::Tag { name, attributes } => self.apply_tag(id, &name, &attributes, source),
        }
    }

    // Apply tag and attribute changes while preserving the existing child nodes.
    fn apply_tag(&mut self, id: i64, before: &str, attributes: &[(String, String)], source: &str) {
        // Accept exactly one opening tag, leaving child nodes outside this edit transaction.
        let Some(parsed) = tag::Tag::parse(source) else {
            self.edit_notice("Enter one complete opening tag; children are edited separately");
            return;
        };
        let name = &source[parsed.name.clone()];
        // Resolve the stable identity before using a row index or applying a retained reply.
        if !valid_tag_name(name) {
            self.edit_notice("Enter a valid element name");
            return;
        }
        // Preserve the subtree: remove omitted attributes, then let Chromium parse
        // the complete attribute text (including entities and quoted values).
        // Removes precede the setter so HTML's case normalization cannot delete
        // a newly supplied attribute with a differently capitalized spelling.
        for (name, _) in attributes {
            // Remove only attributes omitted from the complete draft; retained names are set below.
            if !parsed
                .attributes
                .iter()
                .any(|attr| source[attr.name.clone()] == *name)
            {
                self.command(
                    "DOM.removeAttribute",
                    json!({"nodeId":id,"name":name}),
                    Some(Reply::Edit(self.epoch)),
                );
            }
        }
        let text = &source[parsed.name.end..parsed.end];
        // Skip the attribute setter when only the tag name changed.
        if tag::source(before, attributes, false)[1 + before.len()..]
            .strip_suffix('>')
            .unwrap_or_default()
            != text
        {
            self.command(
                "DOM.setAttributesAsText",
                json!({"nodeId":id,"text":text}),
                Some(Reply::Edit(self.epoch)),
            );
        }
        // Rename last: Chromium returns a new node ID, while attribute commands
        // above still address the original live node. Its children stay intact.
        if name != before {
            self.command(
                "DOM.setNodeName",
                json!({"nodeId":id,"name":name}),
                Some(Reply::Rename(self.epoch, id)),
            );
        }
    }

    // Expose a failed edit without discarding the surrounding inspector state.
    fn edit_notice(&mut self, notice: &str) {
        self.notice = Some(notice.into());
        self.detail_revision += 1;
    }

    // Apply a bounded explicit markup draft to the selected editable node.
    pub fn edit_markup(&mut self, source: &str) {
        // Resolve the stable identity before using a row index or applying a retained reply.
        let Some(id) = self.selected else {
            return;
        };
        // Only complete editable source may enter this mutation path; previews stay read-only.
        if !self.markup_editable() {
            self.edit_notice("Markup preview is truncated; select a smaller element to edit");
            return;
        }
        // Bound the mutation before serializing editable source into a protocol command.
        if source.len() > 65536 {
            self.edit_notice("Edit exceeds the 64 KiB editor limit");
            return;
        }
        let (method, params) = match self.section {
            Section::Html => ("DOM.setOuterHTML", json!({"nodeId":id,"outerHTML":source})),
            Section::Attributes => (
                "DOM.setAttributesAsText",
                json!({"nodeId":id,"text":source}),
            ),
            _ => return,
        };
        self.command(method, params, Some(Reply::Edit(self.epoch)));
    }
}

// Validate a replacement name before any DOM mutation is queued.
fn valid_tag_name(name: &str) -> bool {
    name.chars().enumerate().all(|(index, ch)| {
        ch.is_alphabetic()
            || matches!(ch, '_' | ':')
            || index > 0 && (ch.is_numeric() || matches!(ch, '-' | '.'))
    })
}

#[cfg(test)]
#[path = "../../tests/elements_edit.rs"]
mod tests;
