#![cfg_attr(not(windows), allow(dead_code))]

use crate::{Error, Result};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct RequestIdAllocator {
    next: AtomicU64,
}

impl RequestIdAllocator {
    pub(crate) const fn new() -> Self {
        Self {
            next: AtomicU64::new(0),
        }
    }

    pub(crate) fn next(&self) -> Result<u32> {
        loop {
            let current = self.next.load(Ordering::Relaxed);
            if current > u32::MAX as u64 {
                return Err(Error::IdExhausted);
            }
            if self
                .next
                .compare_exchange_weak(current, current + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(current as u32);
            }
        }
    }

    #[cfg(test)]
    const fn starting_at(next: u64) -> Self {
        Self {
            next: AtomicU64::new(next),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustion_does_not_wrap_or_reuse_request_ids() {
        let ids = RequestIdAllocator::starting_at(u32::MAX as u64);
        assert_eq!(ids.next(), Ok(u32::MAX));
        assert_eq!(ids.next(), Err(Error::IdExhausted));
        assert_eq!(ids.next(), Err(Error::IdExhausted));
    }
}
