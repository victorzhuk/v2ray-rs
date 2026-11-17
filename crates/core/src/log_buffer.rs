use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogSource {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub source: LogSource,
    pub content: String,
}

impl LogLine {
    pub fn stdout(content: impl Into<String>) -> Self {
        Self {
            source: LogSource::Stdout,
            content: content.into(),
        }
    }

    pub fn stderr(content: impl Into<String>) -> Self {
        Self {
            source: LogSource::Stderr,
            content: content.into(),
        }
    }
}

pub struct LogBuffer {
    lines: VecDeque<LogLine>,
    capacity: usize,
}

impl LogBuffer {
    const DEFAULT_CAPACITY: usize = 10_000;

    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, line: LogLine) {
        if self.lines.len() >= self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn last_n(&self, n: usize) -> Vec<&LogLine> {
        let start = self.lines.len().saturating_sub(n);
        self.lines.range(start..).collect()
    }

    pub fn search(&self, query: &str) -> Vec<&LogLine> {
        let query_lower = query.to_lowercase();
        self.lines
            .iter()
            .filter(|line| line.content.to_lowercase().contains(&query_lower))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer() {
        let buffer = LogBuffer::new();
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
        assert_eq!(buffer.last_n(5), Vec::<&LogLine>::new());
        assert_eq!(buffer.search("test"), Vec::<&LogLine>::new());
    }

    #[test]
    fn push_within_capacity() {
        let mut buffer = LogBuffer::with_capacity(5);
        buffer.push(LogLine::stdout("line 1"));
        buffer.push(LogLine::stdout("line 2"));
        buffer.push(LogLine::stderr("line 3"));

        assert_eq!(buffer.len(), 3);
        assert!(!buffer.is_empty());
    }

    #[test]
    fn push_beyond_capacity_evicts_oldest() {
        let mut buffer = LogBuffer::with_capacity(3);
        buffer.push(LogLine::stdout("line 1"));
        buffer.push(LogLine::stdout("line 2"));
        buffer.push(LogLine::stdout("line 3"));
        buffer.push(LogLine::stdout("line 4"));
        buffer.push(LogLine::stdout("line 5"));

        assert_eq!(buffer.len(), 3);
        let lines = buffer.last_n(10);
        assert_eq!(lines[0].content, "line 3");
        assert_eq!(lines[1].content, "line 4");
        assert_eq!(lines[2].content, "line 5");
    }

    #[test]
    fn last_n_returns_requested_lines() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogLine::stdout("line 1"));
        buffer.push(LogLine::stdout("line 2"));
        buffer.push(LogLine::stdout("line 3"));
        buffer.push(LogLine::stdout("line 4"));
        buffer.push(LogLine::stdout("line 5"));

        let last_3 = buffer.last_n(3);
        assert_eq!(last_3.len(), 3);
        assert_eq!(last_3[0].content, "line 3");
        assert_eq!(last_3[1].content, "line 4");
        assert_eq!(last_3[2].content, "line 5");
    }

    #[test]
    fn last_n_with_n_larger_than_buffer() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogLine::stdout("line 1"));
        buffer.push(LogLine::stdout("line 2"));

        let lines = buffer.last_n(100);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn last_n_with_zero() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogLine::stdout("line 1"));

        let lines = buffer.last_n(0);
        assert_eq!(lines.len(), 0);
    }

    #[test]
    fn search_finds_matching_lines() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogLine::stdout("Error: connection failed"));
        buffer.push(LogLine::stdout("Info: starting server"));
        buffer.push(LogLine::stderr("Error: timeout"));
        buffer.push(LogLine::stdout("Debug: processing request"));

        let results = buffer.search("error");
        assert_eq!(results.len(), 2);
        assert!(results[0].content.contains("connection failed"));
        assert!(results[1].content.contains("timeout"));
    }

    #[test]
    fn search_is_case_insensitive() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogLine::stdout("ERROR: failed"));
        buffer.push(LogLine::stdout("Warning: error detected"));
        buffer.push(LogLine::stdout("eRrOr in processing"));

        let results = buffer.search("error");
        assert_eq!(results.len(), 3);

        let results_upper = buffer.search("ERROR");
        assert_eq!(results_upper.len(), 3);
    }

    #[test]
    fn search_no_matches() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogLine::stdout("line 1"));
        buffer.push(LogLine::stdout("line 2"));

        let results = buffer.search("notfound");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn clear_empties_buffer() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogLine::stdout("line 1"));
        buffer.push(LogLine::stdout("line 2"));
        buffer.push(LogLine::stdout("line 3"));

        assert_eq!(buffer.len(), 3);

        buffer.clear();

        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
        assert_eq!(buffer.last_n(10), Vec::<&LogLine>::new());
    }

    #[test]
    fn preserves_log_source() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogLine::stdout("stdout line"));
        buffer.push(LogLine::stderr("stderr line"));

        let lines = buffer.last_n(2);
        assert_eq!(lines[0].source, LogSource::Stdout);
        assert_eq!(lines[1].source, LogSource::Stderr);
    }

    #[test]
    fn default_capacity_is_10000() {
        let buffer = LogBuffer::new();
        assert_eq!(buffer.capacity, 10_000);
    }

    #[test]
    fn custom_capacity_respected() {
        let buffer = LogBuffer::with_capacity(100);
        assert_eq!(buffer.capacity, 100);
    }
}
