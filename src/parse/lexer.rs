use std::ffi::OsStr;
use std::ffi::OsString;

pub use std::io::SeekFrom;

use os_str_bytes::RawOsStr;

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawArgs {
    items: Vec<OsString>,
}

impl RawArgs {
    pub fn cursor(&self) -> ArgCursor {
        ArgCursor::new()
    }

    pub fn next(&self, cursor: &mut ArgCursor) -> Option<ParsedArg<'_>> {
        self.next_os(cursor).map(ParsedArg::new)
    }

    pub fn next_os(&self, cursor: &mut ArgCursor) -> Option<&OsStr> {
        let next = self.items.get(cursor.cursor).map(|s| s.as_os_str());
        cursor.cursor = cursor.cursor.saturating_add(1);
        next
    }

    pub fn peek(&self, cursor: &ArgCursor) -> Option<ParsedArg<'_>> {
        self.peek_os(cursor).map(ParsedArg::new)
    }

    pub fn peek_os(&self, cursor: &ArgCursor) -> Option<&OsStr> {
        self.items.get(cursor.cursor).map(|s| s.as_os_str())
    }

    pub fn remaining(&self, cursor: &mut ArgCursor) -> impl Iterator<Item = &OsStr> {
        let remaining = self.items[cursor.cursor..].iter().map(|s| s.as_os_str());
        cursor.cursor = self.items.len();
        remaining
    }

    pub fn seek(&self, cursor: &mut ArgCursor, pos: SeekFrom) {
        let pos = match pos {
            SeekFrom::Start(pos) => pos,
            SeekFrom::End(pos) => (self.items.len() as i64).saturating_add(pos).max(0) as u64,
            SeekFrom::Current(pos) => (cursor.cursor as i64).saturating_add(pos).max(0) as u64,
        };
        let pos = (pos as usize).min(self.items.len());
        cursor.cursor = pos;
    }

    /// Inject arguments before the [`RawArgs::next`]
    pub fn insert(&mut self, cursor: &ArgCursor, insert_items: &[&str]) {
        self.items.splice(
            cursor.cursor..cursor.cursor,
            insert_items.iter().map(OsString::from),
        );
    }
}

impl<I, T> From<I> for RawArgs
where
    I: Iterator<Item = T>,
    T: Into<OsString>,
{
    fn from(val: I) -> Self {
        Self {
            items: val.map(|x| x.into()).collect(),
        }
    }
}

#[derive(Default, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ArgCursor {
    cursor: usize,
}

impl ArgCursor {
    fn new() -> Self {
        Default::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ParsedArg<'s> {
    inner: std::borrow::Cow<'s, RawOsStr>,
    utf8: Option<&'s str>,
}

impl<'s> ParsedArg<'s> {
    fn new(inner: &'s OsStr) -> Self {
        let utf8 = inner.to_str();
        let inner = RawOsStr::new(inner);
        Self { inner, utf8 }
    }

    pub fn is_stdio(&self) -> bool {
        self.inner.as_ref() == "-"
    }

    pub fn is_escape(&self) -> bool {
        self.inner.as_ref() == "--"
    }

    pub fn is_number(&self) -> bool {
        self.to_value()
            .map(|s| s.parse::<f64>().is_ok())
            .unwrap_or_default()
    }

    /// Treat as a long-flag
    ///
    /// **NOTE:** May return an empty flag.  Check [`ParsedArg::is_escape`] to separately detect `--`.
    pub fn to_long(&self) -> Option<(&RawOsStr, Option<&RawOsStr>)> {
        let remainder = self.inner.as_ref().strip_prefix("--")?;
        let parts = if let Some((p0, p1)) = remainder.split_once("=") {
            (p0, Some(p1))
        } else {
            (remainder, None)
        };
        Some(parts)
    }

    /// Can treat as a long-flag
    ///
    /// **NOTE:** May return an empty flag.  Check [`ParsedArg::is_escape`] to separately detect `--`.
    pub fn is_long(&self) -> bool {
        self.inner.as_ref().starts_with("--")
    }

    /// Treat as a short-flag
    ///
    /// **NOTE:** Maybe return an empty flag.  Check [`ParsedArg::is_stdio`] to separately detect
    /// `-`.
    pub fn to_short(&self) -> Option<ShortFlags<'_>> {
        if let Some(remainder_os) = self.inner.as_ref().strip_prefix('-') {
            if remainder_os.starts_with('-') {
                None
            } else {
                let remainder = self.utf8.map(|s| &s[1..]);
                Some(ShortFlags::new(remainder_os, remainder))
            }
        } else {
            None
        }
    }

    /// Can treat as a short-flag
    ///
    /// **NOTE:** Maybe return an empty flag.  Check [`ParsedArg::is_stdio`] to separately detect
    /// `-`.
    pub fn is_short(&self) -> bool {
        self.inner.as_ref().starts_with('-') && !self.is_long()
    }

    /// Treat as a value
    ///
    /// **NOTE:** May return a flag or an escape.
    pub fn to_value_os(&self) -> &RawOsStr {
        self.inner.as_ref()
    }

    /// Treat as a value
    ///
    /// **NOTE:** May return a flag or an escape.
    pub fn to_value(&self) -> Option<&str> {
        self.utf8
    }

    /// Safely print an argument that may contain non-UTF8 content
    ///
    /// This may perform lossy conversion, depending on the platform. If you would like an implementation which escapes the path please use Debug instead.
    pub fn display(&self) -> impl std::fmt::Display + '_ {
        self.inner.to_str_lossy()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ShortFlags<'s> {
    inner: &'s RawOsStr,
    utf8_prefix: std::str::CharIndices<'s>,
    invalid_suffix: Option<&'s RawOsStr>,
}

impl<'s> ShortFlags<'s> {
    fn new(inner: &'s RawOsStr, utf8: Option<&'s str>) -> Self {
        let (utf8_prefix, invalid_suffix) = if let Some(utf8) = utf8 {
            (utf8, None)
        } else {
            split_nonutf8_once(inner)
        };
        let utf8_prefix = utf8_prefix.char_indices();
        Self {
            inner,
            utf8_prefix,
            invalid_suffix,
        }
    }

    pub fn advance_by(&mut self, n: usize) -> Result<(), usize> {
        for i in 0..n {
            self.next().ok_or(i)?.map_err(|_| i)?;
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.invalid_suffix.is_none() && self.utf8_prefix.as_str().is_empty()
    }

    pub fn is_number(&self) -> bool {
        self.invalid_suffix.is_none() && self.utf8_prefix.as_str().parse::<f64>().is_ok()
    }

    pub fn next(&mut self) -> Option<Result<char, &'s RawOsStr>> {
        if let Some((_, flag)) = self.utf8_prefix.next() {
            return Some(Ok(flag));
        }

        if let Some(suffix) = self.invalid_suffix {
            self.invalid_suffix = None;
            return Some(Err(suffix));
        }

        None
    }

    pub fn value_os(&mut self) -> Option<&'s RawOsStr> {
        if let Some((index, _)) = self.utf8_prefix.next() {
            self.utf8_prefix = "".char_indices();
            self.invalid_suffix = None;
            return Some(&self.inner[index..]);
        }

        if let Some(suffix) = self.invalid_suffix {
            self.invalid_suffix = None;
            return Some(suffix);
        }

        None
    }
}

impl<'s> Iterator for ShortFlags<'s> {
    type Item = Result<char, &'s RawOsStr>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next()
    }
}

fn split_nonutf8_once(b: &RawOsStr) -> (&str, Option<&RawOsStr>) {
    match std::str::from_utf8(b.as_raw_bytes()) {
        Ok(s) => (s, None),
        Err(err) => {
            let (valid, after_valid) = b.split_at(err.valid_up_to());
            let valid = std::str::from_utf8(valid.as_raw_bytes()).unwrap();
            (valid, Some(after_valid))
        }
    }
}
