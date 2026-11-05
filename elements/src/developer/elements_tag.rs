//! Tokenize one opening tag for inline selection and validation; Chromium still parses attribute values.
use std::ops::Range;

pub(super) struct Attribute {
    pub name: Range<usize>,
    pub value: Range<usize>,
    pub whole: Range<usize>,
}
pub(super) struct Tag {
    pub name: Range<usize>,
    pub attributes: Vec<Attribute>,
    pub end: usize,
}
impl Tag {
    // Locate tag and attribute ranges while rejecting incomplete or oversized edit syntax.
    pub fn parse(source: &str) -> Option<Self> {
        let mut cursor = Cursor { source, at: 0 };
        cursor.take(b'<')?;
        let name = cursor.name()?;
        let mut attributes = Vec::new();
        loop {
            let separated = cursor.spaces();
            // Validate source boundaries and delimiters before slicing or interpreting editable text.
            if cursor.byte() == Some(b'>') || cursor.source[cursor.at..].starts_with("/>") {
                break;
            }
            // Attributes require whitespace separation and a bounded count before entering the editor.
            if !separated || attributes.len() == 64 {
                return None;
            }
            attributes.push(cursor.attribute()?);
        }
        let end = cursor.at;
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if cursor.byte() == Some(b'/') {
            cursor.at += 1;
        }
        cursor.take(b'>')?;
        (cursor.at == source.len()).then_some(Self {
            name,
            attributes,
            end,
        })
    }

    // Locate editable values using the UTF-16 offsets expected by AppKit.
    pub fn fields(&self, source: &str) -> Vec<Range<usize>> {
        let (mut byte, mut offset) = (0, 0);
        std::iter::once(self.name.clone())
            .chain(self.attributes.iter().map(|attr| attr.value.clone()))
            .map(|range| {
                // Parsed fields are ordered: count each source segment once instead of rescanning every prefix.
                let start = offset + source[byte..range.start].encode_utf16().count();
                let end = start + source[range.clone()].encode_utf16().count();
                byte = range.end;
                offset = end;
                start..end
            })
            .collect()
    }
}
struct Cursor<'a> {
    source: &'a str,
    at: usize,
}
impl Cursor<'_> {
    // Read the current delimiter without advancing beyond the input.
    fn byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.at).copied()
    }

    // Consume the expected delimiter before advancing the parser.
    fn take(&mut self, byte: u8) -> Option<()> {
        (self.byte()? == byte).then(|| self.at += 1)
    }

    // Skip separator whitespace while remembering whether a boundary was present.
    fn spaces(&mut self) -> bool {
        let start = self.at;
        while self.byte().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.at += 1;
        }
        self.at != start
    }

    // Read one complete name before the next markup delimiter.
    fn name(&mut self) -> Option<Range<usize>> {
        let start = self.at;
        while self.byte().is_some_and(name_byte) {
            self.at += 1;
        }
        (self.at > start).then_some(start..self.at)
    }

    // Locate a name and value without interpreting or changing the source text.
    fn attribute(&mut self) -> Option<Attribute> {
        let name = self.name()?;
        let mut whole = name.clone();
        let after_name = self.at;
        self.spaces();
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if self.byte() != Some(b'=') {
            // Boolean attributes keep their name as the Tab selection target.
            self.at = after_name;
            return Some(Attribute {
                value: name.clone(),
                name,
                whole,
            });
        }
        self.at += 1;
        self.spaces();
        let value = self.value()?;
        whole.end = self.at;
        Some(Attribute { name, value, whole })
    }

    // Read a quoted or unquoted value while preserving its exact source range.
    fn value(&mut self) -> Option<Range<usize>> {
        let quote = self.byte()?;
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if quote == b'\'' || quote == b'"' {
            self.at += 1;
            let start = self.at;
            while self.byte().is_some_and(|byte| byte != quote && byte != 0) {
                self.at += 1;
            }
            let end = self.at;
            self.take(quote)?;
            return Some(start..end);
        }
        let start = self.at;
        while self
            .byte()
            .is_some_and(|byte| !byte.is_ascii_whitespace() && byte != b'>')
        {
            // Validate source boundaries and delimiters before slicing or interpreting editable text.
            if matches!(self.byte()?, b'<' | b'=' | b'\'' | b'"' | b'`' | 0) {
                return None;
            }
            self.at += 1;
        }
        (self.at > start).then_some(start..self.at)
    }
}

// Recognize bytes that can belong to a markup name.
fn name_byte(byte: u8) -> bool {
    !byte.is_ascii_whitespace() && !matches!(byte, b'<' | b'>' | b'/' | b'=' | b'\'' | b'"' | 0)
}

// Translate source byte boundaries to the native editor's UTF-16 offsets.
pub(super) fn utf16_range(source: &str, range: Range<usize>) -> Range<usize> {
    let start = source[..range.start].encode_utf16().count();
    start..start + source[range].encode_utf16().count()
}

// Encode markup-sensitive characters before presenting editable attribute text.
pub(super) fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '\n' => escaped.push_str("&#10;"),
            '\r' => escaped.push_str("&#13;"),
            '\t' => escaped.push_str("&#9;"),
            character => escaped.push(character),
        }
    }
    escaped
}

// Expose the complete editable source rather than the shortened preview.
pub(super) fn source(name: &str, attributes: &[(String, String)], preview: bool) -> String {
    let mut source = format!("<{name}");
    for (name, value) in attributes {
        // Only the tree preview needs clipping; full edit source can borrow the original value.
        let clipped;
        let value = if preview {
            clipped = super::clipped(value, 100);
            &clipped
        } else {
            value
        };
        source.push(' ');
        source.push_str(name);
        source.push_str("=\"");
        source.push_str(&escape(value));
        source.push('"');
    }
    source.push('>');
    source
}

// Only whitespace or the closing delimiter becomes an insertion point. Attribute
// hit ranges are half-open, so clicking the following space never selects its neighbor.
pub(super) fn hit_selection(preview: &str, full: &str, offset: usize) -> Option<Range<usize>> {
    let shown = Tag::parse(preview)?;
    let actual = Tag::parse(full)?;
    let name = utf16_range(preview, shown.name);
    // Validate source boundaries and delimiters before slicing or interpreting editable text.
    if name.contains(&offset) {
        return Some(utf16_range(full, actual.name));
    }
    let mut shown_end = name.end;
    let mut actual_end = utf16_range(full, actual.name.clone()).end;
    for (shown, actual) in shown.attributes.iter().zip(&actual.attributes) {
        let whole = utf16_range(preview, shown.whole.clone());
        let target = utf16_range(full, actual.whole.clone());
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if offset < whole.start {
            let caret = actual_end + offset.saturating_sub(shown_end);
            return Some(caret..caret);
        }
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if whole.contains(&offset) {
            return Some(utf16_range(full, actual.value.clone()));
        }
        shown_end = whole.end;
        actual_end = target.end;
    }
    let caret = actual_end
        .saturating_add(offset.saturating_sub(shown_end))
        .min(full.encode_utf16().count() - 1);
    Some(caret..caret)
}

// Add one space before '>' when entering the final insertion slot. Tab operates
// on the current draft, including newly typed attributes; it never commits a partial tag.
pub(super) fn insertion(source: &mut String) -> Option<Range<usize>> {
    let tag = Tag::parse(source)?;
    // Validate source boundaries and delimiters before slicing or interpreting editable text.
    if !source[..tag.end].ends_with(char::is_whitespace) {
        source.insert(tag.end, ' ');
    }
    let end = source.encode_utf16().count() - 1;
    Some(end..end)
}

// Select the neighboring draft field or create the final attribute insertion slot.
pub(super) fn navigate(
    source: &mut String,
    selection: Range<usize>,
    previous: bool,
) -> Option<Range<usize>> {
    let fields = Tag::parse(source)?.fields(source);
    // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
    if previous {
        return fields
            .into_iter()
            .rev()
            .find(|range| range.start < selection.start);
    }
    fields
        .into_iter()
        .find(|range| range.start > selection.start)
        .or_else(|| insertion(source))
}

#[cfg(test)]
#[path = "../../tests/elements_tag.rs"]
mod tests;
