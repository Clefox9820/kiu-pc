//! calls that take long enough for the player to feel: a run of recompiled
//! code, a syscall or a service request. each one is timed for the frame's
//! stats and named in the log when it takes a frame or more, which says where
//! a stall came from.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use zakuro_cpu::Exit;

use crate::{kernel, System};

/// how long one call can take before the log names it, a frame's worth.
pub(crate) const SLOW_CALL: Duration = Duration::from_millis(20);

/// the log target every slow call is written under.
pub(crate) const TARGET: &str = "zakuro_core::slow";

/// svcSendSyncRequest, a service request, which names itself when slow.
pub(crate) const SEND_SYNC_REQUEST: u32 = 0x32;

/// runs a call, adds the time it took to a frame stats counter and returns
/// its result with that time, reading the clock once on each side.
fn timed<T>(counter: &'static AtomicU64, call: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let result = call();
    let took = started.elapsed();
    counter.fetch_add(took.as_nanos() as u64, Ordering::Relaxed);
    (result, took)
}

/// a duration in milliseconds, for the log.
fn ms(took: Duration) -> f64 {
    took.as_secs_f64() * 1000.0
}

/// the moment a service request started, for `request_done`.
pub(crate) fn start() -> Instant {
    Instant::now()
}

/// names a service request in the log when it took a frame or more.
pub(crate) fn request_done(started: Instant, name: &str, command: u16) {
    let took = started.elapsed();
    if took >= SLOW_CALL {
        log::warn!(target: TARGET, "slow request: {name} command 0x{command:04X} took {:.0} ms", ms(took));
    }
}

impl System {
    /// runs recompiled code from `pc` as `run_recompiled` does, and names the
    /// run in the log, with its thread, length and speed, when it took a frame
    /// or more.
    pub(crate) fn run_recompiled_timed(&mut self, pc: u32, deadline: Option<u64>) -> Option<Option<Exit>> {
        let (instructions, cycles) = (self.recompiled_instructions, self.cpu.cycles);
        let (ran, took) = timed(&zakuro_gpu::stats::RECOMPILED, || self.run_recompiled(deadline));
        if took >= SLOW_CALL {
            let took = ms(took);
            let thread = self.kernel.current().map_or_else(String::new, |t| format!("{} (priority {}, core {})", t.name, t.priority, t.processor_id));
            let (instructions, cycles) = (self.recompiled_instructions - instructions, self.cpu.cycles - cycles);
            log::warn!(
                target: TARGET,
                "recompiled code from {pc:08X} ran {took:.0} ms, to {:08X}, on {thread}, {instructions} instructions, {cycles} cycles, {:.0} MIPS",
                self.cpu.regs[15],
                instructions as f64 / took / 1000.0
            );
        }
        ran
    }

    /// runs a syscall, and names it in the log when it took a frame or more.
    /// a service request is left out here, it names itself, see services.
    pub(crate) fn dispatch_svc_timed(&mut self, number: u32) {
        let ((), took) = timed(&zakuro_gpu::stats::SERVICES, || kernel::svc::dispatch(self, number));
        if number != SEND_SYNC_REQUEST && took >= SLOW_CALL {
            log::warn!(target: TARGET, "slow syscall 0x{number:02X} took {:.0} ms", ms(took));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timed_call_returns_its_result_and_adds_its_time_to_the_counter() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let (result, took) = timed(&COUNTER, || {
            std::thread::sleep(Duration::from_millis(2));
            7
        });
        assert_eq!(result, 7);
        assert!(took >= Duration::from_millis(2));
        assert_eq!(COUNTER.load(Ordering::Relaxed), took.as_nanos() as u64);
    }
}
