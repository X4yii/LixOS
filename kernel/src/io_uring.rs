//! Minimal io_uring-like interface (skeleton).

use alloc::collections::VecDeque;
#[derive(Clone, Copy, Debug, Default)]
pub struct IoUringSqe {
    pub op: u8,
    pub fd: i32,
    pub len: u32,
    pub off: u64,
    pub user_data: u64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct IoUringCqe {
    pub user_data: u64,
    pub res: i32,
    pub flags: u32,
}
pub struct IoRing {
    sq: VecDeque<IoUringSqe>,
    cq: VecDeque<IoUringCqe>,
    sq_entries: u32,
    cq_entries: u32,
}
impl IoRing {
    pub const fn new(sq_entries: u32, cq_entries: u32) -> Self {
        Self { sq: VecDeque::new(), cq: VecDeque::new(), sq_entries, cq_entries }
    }
    pub fn submit(&mut self, sqe: IoUringSqe) -> bool {
        if self.sq.len() as u32 >= self.sq_entries { return false; }
        self.sq.push_back(sqe);
        true
    }
    pub fn complete(&mut self, user_data: u64, res: i32) {
        if (self.cq.len() as u32) < self.cq_entries {
            self.cq.push_back(IoUringCqe { user_data, res, flags: 0 });
        }
    }
    pub fn pop_completion(&mut self) -> Option<IoUringCqe> {
        self.cq.pop_front()
    }
    pub fn pending_submissions(&self) -> usize { self.sq.len() }
}
#[cfg(test)]
mod io_uring_tests {
    use super::*;
    #[test]
    fn submit_and_complete() {
        let mut r = IoRing::new(4,4);
        assert!(r.submit(IoUringSqe { user_data: 42, len: 512, ..Default::default() }));
        assert_eq!(r.pending_submissions(),1);
        r.complete(42, 512);
        let c = r.pop_completion().unwrap();
        assert_eq!(c.user_data,42);
        assert_eq!(c.res,512);
        assert!(r.pop_completion().is_none());
    }
    #[test]
    fn ring_capacity_limits() {
        let mut r = IoRing::new(1,1);
        assert!(r.submit(IoUringSqe::default()));
        assert!(!r.submit(IoUringSqe::default()));
        r.complete(1,1);
        assert!(r.pop_completion().is_some());
    }
}
