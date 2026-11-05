//! Own bounded Console rows, remote-value snapshots and filtering independently of scoped transport.
use super::*;

impl Session {
    // Clear Console rows and release the remote objects they owned.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.objects.clear();
        self.completion.cancel();
        self.changes.completion = true;
    }

    // Clear local rows immediately, but release engine objects only through a connected scoped session.
    pub fn clear_commands(&mut self) -> Vec<String> {
        self.abandoned_evaluations.extend(&self.evaluations);
        // Group cleanup below includes property children; avoid one CDP release per node.
        self.objects.reset();
        self.clear();
        // Disconnected modules must never send root-session cleanup commands to another inspector.
        if !self.ready {
            return vec![];
        }
        let mut commands = self.drain_commands();
        commands.push(self.command(
            protocol_id(),
            "Runtime.releaseObjectGroup",
            json!({"objectGroup":"enkel-native-console"}),
        ));
        // Dropping frontend handles does not discard Chromium's saved console arguments.
        commands.push(self.command(protocol_id(), "Runtime.discardConsoleEntries", json!({})));
        commands
    }

    // Render a bounded Console snapshot using the current filter.
    pub fn view(&self, query: &str) -> ObjectView {
        let query = query.trim().to_lowercase();
        let mut view = ObjectView::default();
        for row in &self.rows {
            // Resolve the live tree identity before reading or mutating its retained state.
            if !query.is_empty()
                && !contains(&row.text, &query)
                && !row
                    .roots
                    .iter()
                    .any(|id| contains(self.objects.description(*id), &query))
            {
                continue;
            }
            // Separate rendered Console rows without introducing an extra initial newline.
            if !row.text.is_empty() && !view.text.is_empty() {
                view.push("\n");
            }
            // Avoid rendering a separate empty text line for rows represented only by value nodes.
            if !row.text.is_empty() {
                view.push(&row.text);
            }
            self.objects.render(&row.roots, &mut view);
            view.finish_row();
        }
        view
    }

    // Convert Console arguments into retained, expandable value rows.
    pub(super) fn push_values(&mut self, prefix: &str, values: &[Value]) {
        let mut text = String::new();
        let mut remaining = MAX_ROW_CHARS;
        append_text(&mut text, prefix, &mut remaining);
        let mut separate = !prefix.is_empty();
        let mut roots = vec![];
        for value in values {
            // Remote handles still need capture/release even after the text budget has been filled.
            if value["objectId"].as_str().is_some()
                && let Some(id) = self.objects.capture("", value, 0)
            {
                roots.push(id);
                continue;
            }
            // Preserve argument separators without building an unbounded intermediate Vec and joined string.
            if separate {
                append_text(&mut text, " ", &mut remaining);
            }
            // A full row needs no more primitive conversions; remote ownership was handled above.
            if remaining > 0 {
                append_remote(&mut text, value, &mut remaining);
            }
            separate = true;
        }
        self.push_row(Row { text, roots });
    }

    // Retain one Console message with its severity and source.
    pub(super) fn push(&mut self, text: impl AsRef<str>) {
        self.push_row(Row {
            text: text.as_ref().chars().take(MAX_ROW_CHARS).collect(),
            roots: vec![],
        });
    }

    // Append a Console row while enforcing the shared retention limit.
    fn push_row(&mut self, row: Row) {
        self.rows.push_back(row);
        while self.rows.len() > MAX_ROWS {
            // Apply the optional result only while its owning state is available.
            if let Some(row) = self.rows.pop_front() {
                self.objects.remove_roots(&row.roots);
            }
        }
    }
}
