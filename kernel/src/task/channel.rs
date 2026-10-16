use crate::sync::IrqSafeSpinlock;
use crate::task::handle::KernelObject;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use lix_sys::{ErrorCode, Rights};
pub struct ChannelMessage {
    pub data: Vec<u8>,
    pub handles: Vec<(KernelObject, Rights)>,
}
pub struct ChannelEndpoint {
    peer: IrqSafeSpinlock<Option<Weak<ChannelEndpoint>>>,
    queue: IrqSafeSpinlock<VecDeque<ChannelMessage>>,
}
impl core::fmt::Debug for ChannelEndpoint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChannelEndpoint").finish()
    }
}
impl ChannelEndpoint {
    pub fn new_pair() -> (Arc<Self>, Arc<Self>) {
        let a = Arc::new(Self {
            peer: IrqSafeSpinlock::new(None),
            queue: IrqSafeSpinlock::new(VecDeque::new()),
        });
        let b = Arc::new(Self {
            peer: IrqSafeSpinlock::new(Some(Arc::downgrade(&a))),
            queue: IrqSafeSpinlock::new(VecDeque::new()),
        });
        *a.peer.lock() = Some(Arc::downgrade(&b));
        (a, b)
    }
    pub fn send(&self, msg: ChannelMessage) -> Result<(), ErrorCode> {
        let peer_weak = self.peer.lock().clone();
        let peer_arc = match peer_weak {
            Some(w) => w.upgrade().ok_or(ErrorCode::ELIX_PEER_CLOSED)?,
            None => return Err(ErrorCode::ELIX_PEER_CLOSED),
        };
        let mut queue = peer_arc.queue.lock();
        if queue.len() >= 256 {
            return Err(ErrorCode::EBUSY);
        }
        queue.push_back(msg);
        Ok(())
    }
    pub fn recv(&self) -> Result<ChannelMessage, ErrorCode> {
        let mut queue = self.queue.lock();
        if let Some(msg) = queue.pop_front() {
            return Ok(msg);
        }
        let peer_weak = self.peer.lock().clone();
        let peer_alive = peer_weak.and_then(|w| w.upgrade()).is_some();
        if !peer_alive {
            Err(ErrorCode::ELIX_PEER_CLOSED)
        } else {
            Err(ErrorCode::EAGAIN)
        }
    }
}
