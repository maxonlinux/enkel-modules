//! Side-effect-checked completion of identifier chains, with stale reply isolation.
use super::protocol_id;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

#[derive(Debug, PartialEq)]
pub struct Query {
    pub expression: String,
    pub start: usize,
    pub caret: usize,
    receiver: String,
    prefix: String,
}
impl Query {
    // Identify the bounded JavaScript expression and replacement range at the caret.
    pub fn parse(expression: &str, caret: usize) -> Option<Self> {
        let byte = utf16_byte(expression, caret)?;
        let before = &expression[..byte];
        // Do not offer identifiers inside quoted text. Template interpolation
        // and comment completion are intentionally outside this first version.
        let mut quote = None;
        let mut escaped = false;
        for c in before.chars() {
            // An escaped quote does not end the current JavaScript string.
            if escaped {
                escaped = false;
                continue;
            }
            // Validate source boundaries and delimiters before slicing or interpreting editable text.
            if quote.is_some() && c == '\\' {
                escaped = true;
                continue;
            }
            // Validate source boundaries and delimiters before slicing or interpreting editable text.
            if Some(c) == quote {
                quote = None;
            } else if quote.is_none() && matches!(c, '\'' | '"' | '`') {
                // Begin a string only outside another quoted region.

                quote = Some(c);
            }
        }
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if quote.is_some() {
            return None;
        }
        let start_byte = before
            .char_indices()
            .rfind(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '$' || *c == '.'))
            .map_or(0, |(index, c)| index + c.len_utf8());
        let chain = &before[start_byte..];
        // An empty property chain means completion should inspect global names.
        if chain.is_empty() {
            return None;
        }
        let (receiver, prefix) = chain.rsplit_once('.').unwrap_or(("", chain));
        // Resolve the stable identity before using a row index or applying a retained reply.
        if !receiver.is_empty() && !receiver.split('.').all(identifier) {
            return None;
        }
        // Resolve the stable identity before using a row index or applying a retained reply.
        if !prefix.is_empty() && !identifier(prefix) {
            return None;
        }
        // Do not open completion for an empty receiver and empty prefix.
        if receiver.is_empty() && prefix.is_empty() {
            return None;
        }
        // Reject calls, indexing and numeric literals rather than executing
        // arbitrary receiver expressions just to discover completion names.
        if start_byte > 0
            && matches!(
                expression[..start_byte].chars().last(),
                Some(')' | ']' | '\'' | '"' | '`')
            )
        {
            return None;
        }
        let start = before.encode_utf16().count() - prefix.encode_utf16().count();
        Some(Self {
            expression: expression.into(),
            start,
            caret,
            receiver: receiver.into(),
            prefix: prefix.into(),
        })
    }

    // Replace only the validated completion range in the captured expression.
    pub fn apply(&self, candidate: &str) -> Option<(String, usize)> {
        let start = utf16_byte(&self.expression, self.start)?;
        let end = utf16_byte(&self.expression, self.caret)?;
        Some((
            format!(
                "{}{}{}",
                &self.expression[..start],
                candidate,
                &self.expression[end..]
            ),
            self.start + candidate.encode_utf16().count(),
        ))
    }
}

// Translate a native caret offset only at a valid Unicode boundary.
pub(crate) fn utf16_byte(value: &str, offset: usize) -> Option<usize> {
    let mut seen = 0;
    for (byte, c) in value.char_indices() {
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if seen == offset {
            return Some(byte);
        }
        seen += c.len_utf16();
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if seen > offset {
            return None;
        }
    }
    (seen == offset).then_some(value.len())
}

// Recognize JavaScript identifier characters for bounded completion parsing.
fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}
#[derive(Debug, PartialEq)]
pub struct Suggestions {
    pub query: Query,
    pub names: Vec<String>,
}
struct Batch {
    token: i32,
    query: Query,
    names: BTreeSet<String>,
    properties: bool,
    lexical: bool,
}
impl Batch {
    // Add names.
    fn add_names<'a>(&mut self, names: impl Iterator<Item = &'a str>) {
        // Keep the same sorted first 40 across both replies, without copying rejected names.
        for name in names.filter(|name| name.starts_with(&self.query.prefix)) {
            // Skip names that cannot enter the bounded sorted suggestion set.
            if self.names.len() == 40 && self.names.last().is_some_and(|last| name >= last.as_str())
            {
                continue;
            }
            self.names.insert(name.into());
            // Keep only the first forty ordered suggestions.
            if self.names.len() > 40 {
                self.names.pop_last();
            }
        }
    }
}
enum Pending {
    Evaluate(i32, String),
    Properties(i32, String),
    Lexical(i32),
}
#[derive(Default)]
pub struct Completion {
    batch: Option<Batch>,
    pending: HashMap<i32, Pending>,
    pub suggestions: Option<Rc<Suggestions>>,
    commands: Vec<Value>,
}
impl Completion {
    // Retire in-flight completion and release any retained remote object.
    pub fn cancel(&mut self) {
        self.batch = None;
        self.suggestions = None;
    }

    // Begin completion only when the expression can be inspected safely.
    pub fn start(&mut self, expression: &str, caret: usize) {
        self.cancel();
        // Bound retained data and per-event work before accepting more content.
        if self.pending.len() >= 64 {
            return;
        }
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some(query) = Query::parse(expression, caret) else {
            return;
        };
        let token = protocol_id();
        let group = format!("enkel-completion-{token}");
        let global = query.receiver.is_empty();
        // Global completion needs lexical bindings as well as object properties.
        let expression = if global {
            "globalThis".into()
        } else {
            format!("Object({})", query.receiver)
        };
        self.batch = Some(Batch {
            token,
            query,
            names: BTreeSet::new(),
            properties: false,
            lexical: !global,
        });
        self.pending
            .insert(token, Pending::Evaluate(token, group.clone()));
        self.commands.push(json!({"id":token,"method":"Runtime.evaluate","params":{
            "expression":expression,"objectGroup":group,"silent":true,"throwOnSideEffect":true,"timeout":100}}));
        // Property completion has no global lexical batch to wait for.
        if !global {
            return;
        }
        let id = protocol_id();
        self.pending.insert(id, Pending::Lexical(token));
        self.commands
            .push(json!({"id":id,"method":"Runtime.globalLexicalScopeNames"}));
    }

    // Route incoming protocol data to its owner and ignore replies from stale work.
    pub fn receive(&mut self, message: &Value) -> bool {
        // Route only the protocol event or reply owned by this handler.
        let Some(id) = message["id"].as_i64().and_then(|id| i32::try_from(id).ok()) else {
            return false;
        };
        // Avoid duplicate in-flight work and wait for the reply that already owns this state.
        let Some(pending) = self.pending.remove(&id) else {
            return false;
        };
        match pending {
            Pending::Evaluate(token, group) => self.evaluated(token, group, message),
            Pending::Properties(token, group) => self.properties(token, group, message),
            Pending::Lexical(token) => self.lexical(token, message),
        }
        self.finish_batch();
        true
    }

    // Match an evaluation reply to the expression that requested it.
    fn evaluated(&mut self, token: i32, group: String, message: &Value) {
        let current = self
            .batch
            .as_ref()
            .is_some_and(|batch| batch.token == token);
        let object = message["result"]["result"]["objectId"].as_str();
        // Route only the protocol event or reply owned by this handler.
        if current
            && message["result"].get("exceptionDetails").is_none()
            && let Some(object) = object
        {
            let id = protocol_id();
            self.pending.insert(id, Pending::Properties(token, group));
            self.commands.push(json!({"id":id,"method":"Runtime.getProperties","params":{"objectId":object,"ownProperties":false}}));
            return;
        }
        self.release(group);
        // Apply the optional result only while its owning state is available.
        if let Some(batch) = self.batch.as_mut().filter(|batch| batch.token == token) {
            batch.properties = true;
        }
    }

    // Collect property-name suggestions without invoking object getters.
    fn properties(&mut self, token: i32, group: String, message: &Value) {
        self.release(group);
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some(batch) = self.batch.as_mut().filter(|batch| batch.token == token) else {
            return;
        };
        batch.properties = true;
        batch.add_names(
            message["result"]["result"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|property| property["name"].as_str().filter(|name| identifier(name))),
        );
    }

    // Collect global lexical names for the current completion query.
    fn lexical(&mut self, token: i32, message: &Value) {
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some(batch) = self.batch.as_mut().filter(|batch| batch.token == token) else {
            return;
        };
        batch.lexical = true;
        batch.add_names(
            message["result"]["names"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
    }

    // Finish batch.
    fn finish_batch(&mut self) {
        // Publish global suggestions only after both property and lexical replies arrive.
        if !self
            .batch
            .as_ref()
            .is_some_and(|batch| batch.properties && batch.lexical)
        {
            return;
        }
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some(batch) = self.batch.take() else {
            return;
        };
        let names = batch.names.into_iter().collect::<Vec<_>>();
        // Do not show an empty popup or a single suggestion identical to the typed prefix.
        if !names.is_empty() && !(names.len() == 1 && names[0] == batch.query.prefix) {
            self.suggestions = Some(Rc::new(Suggestions {
                query: batch.query,
                names,
            }));
        }
    }

    // Queue release of remote objects when their retained owner is finished.
    fn release(&mut self, group: String) {
        self.commands.push(json!({"id":protocol_id(),"method":"Runtime.releaseObjectGroup","params":{"objectGroup":group}}));
    }

    // Transfer queued commands to the caller without keeping a second copy.
    pub fn drain(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.commands)
    }

    // Discard suggestions and their pending protocol ownership.
    pub fn reset(&mut self) {
        self.cancel();
        self.pending.clear();
        self.commands.clear();
    }
}

/// Session-only command history; do not persist expressions that may contain secrets.
#[derive(Default)]
pub struct CommandHistory {
    entries: std::collections::VecDeque<String>,
    cursor: Option<usize>,
    draft: String,
}
impl CommandHistory {
    // Keep bounded command history without adjacent duplicate submissions.
    pub fn record(&mut self, expression: &str) {
        self.cursor = None;
        self.draft.clear();
        // Ignore blank commands and adjacent duplicate history entries.
        if expression.trim().is_empty()
            || self.entries.back().is_some_and(|last| last == expression)
        {
            return;
        }
        self.entries.push_back(expression.into());
        // Bound retained command history to the most recent hundred submissions.
        if self.entries.len() > 100 {
            self.entries.pop_front();
        }
    }

    // Move through the current source or history without inventing a new target.
    pub fn navigate(&mut self, previous: bool, current: &str) -> Option<String> {
        // An empty history cannot replace the current draft.
        if self.entries.is_empty() {
            return None;
        }
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        let next = if previous {
            match self.cursor {
                None => {
                    self.draft = current.into();
                    self.entries.len() - 1
                }
                Some(index) => index.saturating_sub(1),
            }
        } else {
            let index = self.cursor? + 1;
            // Moving past the newest command restores the unsent draft.
            if index >= self.entries.len() {
                self.cursor = None;
                return Some(std::mem::take(&mut self.draft));
            }
            index
        };
        self.cursor = Some(next);
        self.entries.get(next).cloned()
    }
}

// Share the completion parser's UTF-16 boundary conversion; never allocate a
// second code buffer just to inspect a native caret or retain indentation.
pub(crate) fn history_boundary(value: &str, selection: std::ops::Range<usize>, up: bool) -> bool {
    // Keep the caret and selection on valid native UTF-16/source boundaries.
    let Some(caret) = utf16_byte(value, selection.start) else {
        return false;
    };
    // History starts at the first line for Up and the last line for Down.
    let segment = if up { &value[..caret] } else { &value[caret..] };
    selection.is_empty() && !segment.contains('\n')
}

// Preserve leading indentation when inserting a line into the Console draft.
pub(crate) fn indented_newline(value: &str, caret: usize) -> String {
    let caret = utf16_byte(value, caret).unwrap_or(value.len());
    let line = value[..caret].rsplit('\n').next().unwrap_or_default();
    let length = line.len() - line.trim_start_matches([' ', '\t']).len();
    format!("\n{}", &line[..length])
}

#[cfg(test)]
#[path = "../../tests/completion.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/console_input.rs"]
mod input_tests;
