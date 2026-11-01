//! Reservations for queued, handshaking and active peer connections.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use parking_lot::Mutex;

const MAX_PER_IP: usize = 4;

#[derive(Default)]
struct Counts {
    total: usize,
    by_ip: HashMap<IpAddr, usize>,
}

pub(crate) struct Admission {
    limit: usize,
    counts: Mutex<Counts>,
}

/// Own this for the entire connection, including time spent waiting in a queue.
pub(crate) struct Permit {
    admission: Arc<Admission>,
    ip: Option<IpAddr>,
}

impl Admission {
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            counts: Mutex::default(),
        })
    }

    pub(crate) fn try_acquire(self: &Arc<Self>, addr: Option<SocketAddr>) -> Option<Permit> {
        let ip = addr.map(|addr| addr.ip().to_canonical());
        let mut counts = self.counts.lock();
        if counts.total >= self.limit
            || ip.is_some_and(|ip| counts.by_ip.get(&ip).copied().unwrap_or(0) >= MAX_PER_IP)
        {
            return None;
        }
        counts.total += 1;
        if let Some(ip) = ip {
            *counts.by_ip.entry(ip).or_default() += 1;
        }
        Some(Permit {
            admission: self.clone(),
            ip,
        })
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut counts = self.admission.counts.lock();
        counts.total -= 1;
        if let Some(ip) = self.ip {
            let count = counts.by_ip.get_mut(&ip).expect("reserved source IP");
            *count -= 1;
            if *count == 0 {
                counts.by_ip.remove(&ip);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_limits_cover_ports_and_mapped_ipv4_and_release_on_drop() {
        let admission = Admission::new(50);
        let mut held = Vec::new();
        for port in 1..=4 {
            held.push(
                admission
                    .try_acquire(Some(([1, 2, 3, 4], port).into()))
                    .unwrap(),
            );
        }
        assert!(
            admission
                .try_acquire(Some("[::ffff:1.2.3.4]:9999".parse().unwrap()))
                .is_none()
        );
        assert!(
            admission
                .try_acquire(Some(([1, 2, 3, 5], 1).into()))
                .is_some()
        );
        held.pop();
        assert!(
            admission
                .try_acquire(Some("[::ffff:1.2.3.4]:9999".parse().unwrap()))
                .is_some()
        );
        drop(held);
        assert_eq!(admission.counts.lock().total, 0);
        assert!(admission.counts.lock().by_ip.is_empty());
    }

    #[tokio::test]
    async fn shared_capacity_is_released_by_aborting_a_connection() {
        let admission = Admission::new(2);
        let one = admission.try_acquire(None).unwrap();
        let two = admission
            .try_acquire(Some(([1, 2, 3, 4], 1).into()))
            .unwrap();
        assert!(admission.try_acquire(None).is_none());
        let task = tokio::spawn(async move {
            let _held = (one, two);
            std::future::pending::<()>().await;
        });
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let _one = admission.try_acquire(None).unwrap();
        let _two = admission
            .try_acquire(Some(([1, 2, 3, 4], 1).into()))
            .unwrap();
        assert!(admission.try_acquire(None).is_none());
    }
}
