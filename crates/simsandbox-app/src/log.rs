//! The message log: notices, warnings and errors, newest last. The status
//! tile shows the newest; the whole log opens from it.

use std::time::{Duration, Instant};

/// How long a new entry stands out in the status tile before it dims.
pub const FRESH_FOR: Duration = Duration::from_secs(8);

/// At most this many entries are kept; older ones are dropped.
const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    pub text: String,
    pub at: Instant,
}

impl Entry {
    /// Shown in full colour: new, or an error still in force.
    pub fn is_fresh(&self) -> bool {
        self.at.elapsed() < FRESH_FOR
    }
}

#[derive(Default)]
pub struct Log {
    entries: Vec<Entry>,
    /// Entries added since the log was last opened.
    unread: usize,
}

impl Log {
    /// Add an entry. The same message again (e.g. while a slider is
    /// dragged) only refreshes the newest entry's time.
    pub fn push(&mut self, kind: Kind, text: impl Into<String>) {
        let text = text.into();
        if let Some(last) = self.entries.last_mut() {
            if last.kind == kind && last.text == text {
                last.at = Instant::now();
                return;
            }
        }
        self.entries.push(Entry {
            kind,
            text,
            at: Instant::now(),
        });
        self.unread += 1;
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.push(Kind::Info, text);
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(Kind::Warning, text);
    }

    pub fn latest(&self) -> Option<&Entry> {
        self.entries.last()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn unread(&self) -> usize {
        self.unread
    }

    pub fn mark_read(&mut self) {
        self.unread = 0;
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.unread = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_entries_and_counts_unread() {
        let mut log = Log::default();
        assert!(log.latest().is_none());
        for k in 0..MAX_ENTRIES + 5 {
            log.info(format!("n{k}"));
        }
        log.warn("last");
        log.warn("last");
        assert_eq!(log.entries().len(), MAX_ENTRIES);
        assert_eq!(log.entries()[0].text, "n6");
        assert_eq!(log.latest().unwrap().kind, Kind::Warning);
        assert!(log.latest().unwrap().is_fresh());
        assert_eq!(log.unread(), MAX_ENTRIES + 6);
        log.mark_read();
        assert_eq!(log.unread(), 0);
        log.clear();
        assert!(log.entries().is_empty());
    }
}
