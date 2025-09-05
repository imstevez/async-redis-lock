use std::time::Duration;

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1000);
const DEFAULT_RETRY: Duration = Duration::from_millis(100);
const DEFAULT_TTL: Duration = Duration::from_millis(3000);
const DEFAULT_EXTEND: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub retry: Duration,
    pub timeout: Option<Duration>,
    pub ttl: Duration,
    pub extend: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            retry: DEFAULT_RETRY,
            timeout: Some(DEFAULT_TIMEOUT),
            ttl: DEFAULT_TTL,
            extend: DEFAULT_EXTEND,
        }
    }
}

impl Options {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn retry(mut self, dur: Duration) -> Self {
        self.retry = dur;
        self
    }

    pub fn timeout(mut self, dur: Option<Duration>) -> Self {
        self.timeout = dur;
        self
    }

    pub fn ttl(mut self, dur: Duration) -> Self {
        self.ttl = dur;
        self
    }

    pub fn extend(mut self, dur: Duration) -> Self {
        self.extend = dur;
        self
    }
}
