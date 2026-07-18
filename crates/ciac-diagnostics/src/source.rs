use serde::Serialize;

/// Identifies a file registered in a [`SourceMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct FileId(pub u32);

/// A byte range within a single source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: u32, end: u32) -> Self {
        debug_assert!(start <= end);
        Self { file, start, end }
    }

    /// A span covering both `self` and `other` (same file).
    pub fn to(self, other: Span) -> Span {
        debug_assert_eq!(self.file, other.file);
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

/// A single registered source file.
#[derive(Debug)]
pub struct SourceFile {
    pub name: String,
    pub src: String,
    /// Byte offsets of the start of each line, for line/column lookup.
    line_starts: Vec<u32>,
}

impl SourceFile {
    fn new(name: String, src: String) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        Self {
            name,
            src,
            line_starts,
        }
    }

    /// 1-based (line, column) of a byte offset.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,
            Err(idx) => idx - 1,
        };
        let col = offset - self.line_starts[line];
        (line as u32 + 1, col + 1)
    }

    /// The inverse of [`SourceFile::line_col`] (v0.18 M4, position-based
    /// rename): the byte offset of a 1-based `(line, column)`, or `None`
    /// when either is out of range. `column` is a byte offset within the
    /// line, matching `line_col`'s own byte-oriented convention (CIaC
    /// identifiers are ASCII, so this never needs UTF-16/UTF-8 width
    /// reconciliation the way an LSP-facing position would).
    pub fn offset_of(&self, line: u32, column: u32) -> Option<u32> {
        let line_start = *self.line_starts.get((line as usize).checked_sub(1)?)?;
        let offset = line_start + column.checked_sub(1)?;
        (offset as usize <= self.src.len()).then_some(offset)
    }
}

/// Registry of all source files participating in a compilation.
#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, name: impl Into<String>, src: impl Into<String>) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(SourceFile::new(name.into(), src.into()));
        id
    }

    pub fn file(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }

    pub fn source(&self, id: FileId) -> &str {
        &self.file(id).src
    }

    pub fn snippet(&self, span: Span) -> &str {
        &self.source(span.file)[span.range()]
    }

    /// Every registered file, in registration order (for a module-
    /// resolved program, that's import resolution order — deterministic
    /// and suitable for hashing the whole source set).
    pub fn files(&self) -> impl Iterator<Item = &SourceFile> {
        self.files.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_lookup() {
        let mut map = SourceMap::new();
        let id = map.add_file("test.ciac", "abc\ndef\n");
        let file = map.file(id);
        assert_eq!(file.line_col(0), (1, 1));
        assert_eq!(file.line_col(3), (1, 4));
        assert_eq!(file.line_col(4), (2, 1));
        assert_eq!(file.line_col(6), (2, 3));
    }

    #[test]
    fn span_join_and_snippet() {
        let mut map = SourceMap::new();
        let id = map.add_file("test.ciac", "service Video;");
        let a = Span::new(id, 0, 7);
        let b = Span::new(id, 8, 13);
        assert_eq!(map.snippet(a.to(b)), "service Video");
    }
}
