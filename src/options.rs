use std::time::Duration;

const DEFAULT_DURATION: Duration = Duration::from_millis(3000);
const DEFAULT_RETRY_INTERVAL: Duration = Duration::from_millis(500);
const DEFAULT_RETRY_TIMEOUT: Duration = Duration::from_millis(1000);
const DEFAULT_EXTEND_INTERVAL: Duration = Duration::from_millis(2000);

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub lifetime: Duration,
    pub retry_interval: Duration,
    pub retry_timeout: Option<Duration>,
    pub extend_interval: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            lifetime: DEFAULT_DURATION,
            retry_interval: DEFAULT_RETRY_INTERVAL,
            retry_timeout: Some(DEFAULT_RETRY_TIMEOUT),
            extend_interval: DEFAULT_EXTEND_INTERVAL,
        }
    }
}

impl Options {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lifetime(mut self, dur: Duration) -> Self {
        self.lifetime = dur;
        self
    }

    pub fn retry_interval(mut self, dur: Duration) -> Self {
        self.retry_interval = dur;
        self
    }

    pub fn retry_timeout(mut self, dur: Option<Duration>) -> Self {
        self.retry_timeout = dur;
        self
    }

    pub fn extend_interval(mut self, dur: Duration) -> Self {
        self.extend_interval = dur;
        self
    }
}
