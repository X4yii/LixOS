use crate::sync::IrqSafeSpinlock;
use alloc::collections::VecDeque;
pub use lix_sys::InputEvent;
const QUEUE_LIMIT: usize = 1024;
static EVENTS: IrqSafeSpinlock<VecDeque<InputEvent>> = IrqSafeSpinlock::new(VecDeque::new());
pub fn push(event: InputEvent) {
    let mut events = EVENTS.lock();
    if events.len() == QUEUE_LIMIT {
        events.pop_front();
    }
    events.push_back(event);
}
pub fn push_usb(device: u16, event: lix_usb::InputEvent) {
    push(InputEvent {
        time_ns: crate::task::scheduler::total_ticks().saturating_mul(10_000_000),
        device,
        kind: event.kind,
        code: event.code,
        value: event.value,
    });
}
pub fn pop() -> Option<InputEvent> {
    EVENTS.lock().pop_front()
}
pub fn pop_for_class(class: u32) -> Option<InputEvent> {
    let mut events = EVENTS.lock();
    let position = events.iter().position(|event| match class {
        0 => true,
        1 => event.kind == lix_usb::INPUT_KEY,
        2 => event.kind != lix_usb::INPUT_KEY,
        _ => false,
    })?;
    events.remove(position)
}
pub fn drain(out: &mut [InputEvent]) -> usize {
    let mut events = EVENTS.lock();
    let mut count = 0;
    while count < out.len() {
        let Some(event) = events.pop_front() else {
            break;
        };
        out[count] = event;
        count += 1;
    }
    count
}
