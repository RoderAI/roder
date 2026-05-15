use std::collections::VecDeque;

use roder_api::tasks::TaskOutputStream;
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLogEntry {
    pub stream: TaskOutputStream,
    pub chunk: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct BoundedLogBuffer {
    max_bytes: usize,
    entries: VecDeque<TaskLogEntry>,
    current_bytes: usize,
    dropped_bytes: u64,
}

impl BoundedLogBuffer {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            entries: VecDeque::new(),
            current_bytes: 0,
            dropped_bytes: 0,
        }
    }

    pub fn push(&mut self, stream: TaskOutputStream, mut chunk: String) -> u64 {
        let mut newly_dropped = 0_u64;
        if self.max_bytes == 0 {
            newly_dropped = chunk.len() as u64;
            self.dropped_bytes += newly_dropped;
            return newly_dropped;
        }

        let chunk_len = chunk.len();
        if chunk_len > self.max_bytes {
            let drop_len = chunk_len - self.max_bytes;
            chunk = chunk[drop_len..].to_string();
            newly_dropped += drop_len as u64;
        }

        self.current_bytes += chunk.len();
        self.entries.push_back(TaskLogEntry {
            stream,
            chunk,
            timestamp: OffsetDateTime::now_utc(),
        });

        while self.current_bytes > self.max_bytes {
            let Some(front) = self.entries.pop_front() else {
                break;
            };
            let front_len = front.chunk.len();
            self.current_bytes = self.current_bytes.saturating_sub(front_len);
            newly_dropped += front_len as u64;
        }

        self.dropped_bytes += newly_dropped;
        newly_dropped
    }

    pub fn entries(&self) -> Vec<TaskLogEntry> {
        self.entries.iter().cloned().collect()
    }

    pub fn dropped_bytes(&self) -> u64 {
        self.dropped_bytes
    }

    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_log_buffer_drops_old_entries() {
        let mut buffer = BoundedLogBuffer::new(8);
        assert_eq!(buffer.push(TaskOutputStream::Stdout, "abc".to_string()), 0);
        assert_eq!(buffer.push(TaskOutputStream::Stdout, "def".to_string()), 0);
        assert_eq!(buffer.push(TaskOutputStream::Stdout, "ghi".to_string()), 3);

        let entries = buffer.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].chunk, "def");
        assert_eq!(entries[1].chunk, "ghi");
        assert_eq!(buffer.dropped_bytes(), 3);
        assert_eq!(buffer.current_bytes(), 6);
    }

    #[test]
    fn bounded_log_buffer_trims_oversized_chunks() {
        let mut buffer = BoundedLogBuffer::new(4);
        assert_eq!(
            buffer.push(TaskOutputStream::Stderr, "abcdef".to_string()),
            2
        );

        let entries = buffer.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].chunk, "cdef");
        assert_eq!(buffer.dropped_bytes(), 2);
        assert_eq!(buffer.current_bytes(), 4);
    }
}
