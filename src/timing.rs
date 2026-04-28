// Provides high-precision sleep and scheduler timing calibration.

use std::ffi::c_void;
use std::hint::spin_loop;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, yield_now};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CancelWaitableTimer, CreateWaitableTimerExW,
    GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
    ProcessPowerThrottling, SetProcessInformation, SetWaitableTimerEx, TIMER_ALL_ACCESS,
    WaitForSingleObject,
};
use windows::core::PCWSTR;

const DEFAULT_SLEEP_GRANULARITY_MS: u64 = 2;
const DEFAULT_SCHEDULER_INTERVAL_MS: u64 = 2;
const PROBE_SLEEP: Duration = Duration::from_millis(1);
const SAMPLE_COUNT: usize = 12;
const SAMPLE_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const SPIN_THRESHOLD: Duration = Duration::from_micros(500);
const YIELD_THRESHOLD: Duration = Duration::from_micros(100);
const TIMER_RESOLUTION_MS: u32 = 1;

#[derive(Clone)]
pub struct SleepTimingMonitor {
    state: Arc<SleepTimingState>,
}

struct SleepTimingState {
    measured_granularity_ms: AtomicU64,
    scheduler_interval_ms: AtomicU64,
    timer_resolution_requested: AtomicBool,
    occlusion_workaround_enabled: AtomicBool,
    high_resolution_waitable_timer: AtomicBool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SleepTimingSnapshot {
    pub measured_granularity_ms: u64,
    pub scheduler_interval_ms: u64,
    pub timer_resolution_requested: bool,
    pub occlusion_workaround_enabled: bool,
    pub high_resolution_waitable_timer: bool,
}

#[derive(Clone)]
pub struct HighPrecisionSleeper {
    timer: Arc<WaitableTimer>,
    _request: Arc<TimerResolutionRequest>,
}

impl SleepTimingMonitor {
    pub fn shared() -> Self {
        static MONITOR: OnceLock<SleepTimingMonitor> = OnceLock::new();
        MONITOR
            .get_or_init(|| {
                let request = timer_resolution_request();
                let state = Arc::new(SleepTimingState {
                    measured_granularity_ms: AtomicU64::new(DEFAULT_SLEEP_GRANULARITY_MS),
                    scheduler_interval_ms: AtomicU64::new(DEFAULT_SCHEDULER_INTERVAL_MS),
                    timer_resolution_requested: AtomicBool::new(request.timer_resolution_requested),
                    occlusion_workaround_enabled: AtomicBool::new(
                        request.occlusion_workaround_enabled,
                    ),
                    high_resolution_waitable_timer: AtomicBool::new(false),
                });
                spawn_sampler(Arc::clone(&state));
                Self { state }
            })
            .clone()
    }

    pub fn snapshot(&self) -> SleepTimingSnapshot {
        SleepTimingSnapshot {
            measured_granularity_ms: self
                .state
                .measured_granularity_ms
                .load(Ordering::Relaxed)
                .max(1),
            scheduler_interval_ms: self
                .state
                .scheduler_interval_ms
                .load(Ordering::Relaxed)
                .max(1),
            timer_resolution_requested: self
                .state
                .timer_resolution_requested
                .load(Ordering::Relaxed),
            occlusion_workaround_enabled: self
                .state
                .occlusion_workaround_enabled
                .load(Ordering::Relaxed),
            high_resolution_waitable_timer: self
                .state
                .high_resolution_waitable_timer
                .load(Ordering::Relaxed),
        }
    }
}

impl HighPrecisionSleeper {
    pub fn new() -> Self {
        let request = timer_resolution_request();
        Self {
            timer: Arc::new(WaitableTimer::new()),
            _request: request,
        }
    }

    pub fn is_high_resolution_waitable_timer(&self) -> bool {
        self.timer.high_resolution
    }

    pub fn sleep_for(&self, duration: Duration) {
        if duration.is_zero() {
            return;
        }
        self.sleep_until(Instant::now() + duration);
    }

    pub fn sleep_until(&self, deadline: Instant) {
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }

            let remaining = deadline.saturating_duration_since(now);
            if remaining <= SPIN_THRESHOLD {
                busy_wait_until(deadline);
                break;
            }

            let wait_duration = remaining.saturating_sub(SPIN_THRESHOLD);
            if wait_duration.is_zero() {
                busy_wait_until(deadline);
                break;
            }

            self.timer.wait(wait_duration);
        }
    }
}

struct WaitableTimer {
    handle: HANDLE,
    high_resolution: bool,
}

impl WaitableTimer {
    fn new() -> Self {
        if let Some(handle) = create_waitable_timer(CREATE_WAITABLE_TIMER_HIGH_RESOLUTION) {
            return Self {
                handle,
                high_resolution: true,
            };
        }

        let handle = create_waitable_timer(0).unwrap_or_default();
        Self {
            handle,
            high_resolution: false,
        }
    }

    fn wait(&self, duration: Duration) {
        if self.handle.is_invalid() {
            thread::sleep(duration);
            return;
        }

        let due_time = relative_due_time(duration);
        let result = unsafe { SetWaitableTimerEx(self.handle, &due_time, 0, None, None, None, 0) };
        if result.is_err() {
            thread::sleep(duration);
            return;
        }

        let wait_result = unsafe { WaitForSingleObject(self.handle, u32::MAX) };
        if wait_result != WAIT_OBJECT_0 {
            thread::sleep(duration);
        }
    }
}

impl Drop for WaitableTimer {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            unsafe {
                let _ = CancelWaitableTimer(self.handle);
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

struct TimerResolutionRequest {
    timer_resolution_requested: bool,
    occlusion_workaround_enabled: bool,
}

impl Drop for TimerResolutionRequest {
    fn drop(&mut self) {
        if self.timer_resolution_requested {
            unsafe {
                let _ = timeEndPeriod(TIMER_RESOLUTION_MS);
            }
        }
    }
}

fn timer_resolution_request() -> Arc<TimerResolutionRequest> {
    static REQUEST: OnceLock<Arc<TimerResolutionRequest>> = OnceLock::new();
    REQUEST
        .get_or_init(|| {
            let timer_resolution_requested = unsafe { timeBeginPeriod(TIMER_RESOLUTION_MS) } == 0;
            let occlusion_workaround_enabled = disable_timer_resolution_occlusion_throttling();
            Arc::new(TimerResolutionRequest {
                timer_resolution_requested,
                occlusion_workaround_enabled,
            })
        })
        .clone()
}

fn spawn_sampler(state: Arc<SleepTimingState>) {
    thread::Builder::new()
        .name("sleep-timing-sampler".to_string())
        .spawn(move || {
            let sleeper = HighPrecisionSleeper::new();
            state.high_resolution_waitable_timer.store(
                sleeper.is_high_resolution_waitable_timer(),
                Ordering::Relaxed,
            );

            loop {
                let snapshot = sample_sleep_timing(&sleeper);
                state
                    .measured_granularity_ms
                    .store(snapshot.measured_granularity_ms, Ordering::Relaxed);
                state
                    .scheduler_interval_ms
                    .store(snapshot.scheduler_interval_ms, Ordering::Relaxed);
                sleeper.sleep_for(SAMPLE_REFRESH_INTERVAL);
            }
        })
        .expect("failed to spawn sleep timing sampler");
}

fn sample_sleep_timing(sleeper: &HighPrecisionSleeper) -> SleepTimingSnapshot {
    let mut samples_us = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        let started_at = Instant::now();
        sleeper.sleep_for(PROBE_SLEEP);
        samples_us.push(started_at.elapsed().as_micros() as u64);
    }

    let mut snapshot = classify_samples(&samples_us);
    let request = timer_resolution_request();
    snapshot.timer_resolution_requested = request.timer_resolution_requested;
    snapshot.occlusion_workaround_enabled = request.occlusion_workaround_enabled;
    snapshot.high_resolution_waitable_timer = sleeper.is_high_resolution_waitable_timer();
    snapshot
}

fn classify_samples(samples_us: &[u64]) -> SleepTimingSnapshot {
    if samples_us.is_empty() {
        return SleepTimingSnapshot {
            measured_granularity_ms: DEFAULT_SLEEP_GRANULARITY_MS,
            scheduler_interval_ms: DEFAULT_SCHEDULER_INTERVAL_MS,
            timer_resolution_requested: false,
            occlusion_workaround_enabled: false,
            high_resolution_waitable_timer: false,
        };
    }

    let mut sorted = samples_us.to_vec();
    sorted.sort_unstable();

    let median_us = percentile_us(&sorted, 50);
    let p10_us = percentile_us(&sorted, 10);
    let p90_us = percentile_us(&sorted, 90);
    let jitter_us = p90_us.saturating_sub(p10_us);

    let measured_granularity_ms = ceil_div_u64(median_us.max(1), 1_000).max(1);
    let safety_margin_ms = match jitter_us {
        0..=200 => 0,
        201..=700 => 1,
        _ => 2,
    };
    let scheduler_interval_ms = measured_granularity_ms
        .saturating_add(safety_margin_ms)
        .max(1);

    SleepTimingSnapshot {
        measured_granularity_ms,
        scheduler_interval_ms,
        timer_resolution_requested: false,
        occlusion_workaround_enabled: false,
        high_resolution_waitable_timer: false,
    }
}

fn busy_wait_until(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }

        let remaining = deadline.saturating_duration_since(now);
        if remaining > YIELD_THRESHOLD {
            yield_now();
        } else {
            spin_loop();
        }
    }
}

fn create_waitable_timer(flags: u32) -> Option<HANDLE> {
    unsafe { CreateWaitableTimerExW(None, PCWSTR::null(), flags, TIMER_ALL_ACCESS.0) }.ok()
}

fn disable_timer_resolution_occlusion_throttling() -> bool {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0,
    };

    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &state as *const PROCESS_POWER_THROTTLING_STATE as *const c_void,
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .is_ok()
    }
}

fn relative_due_time(duration: Duration) -> i64 {
    let nanos = duration.as_nanos().max(100);
    let ticks_100ns = nanos.div_ceil(100).min(i64::MAX as u128);
    -(ticks_100ns as i64)
}

fn percentile_us(samples_us: &[u64], percentile: usize) -> u64 {
    let index = ((samples_us.len().saturating_sub(1)) * percentile) / 100;
    samples_us[index]
}

fn ceil_div_u64(value: u64, divisor: u64) -> u64 {
    value.div_ceil(divisor)
}

#[cfg(test)]
mod tests {
    use super::{SleepTimingSnapshot, classify_samples, relative_due_time};
    use std::time::Duration;

    #[test]
    fn stable_samples_reduce_margin() {
        let snapshot = classify_samples(&[1_040, 1_050, 1_060, 1_070, 1_080, 1_090]);
        assert_eq!(
            snapshot,
            SleepTimingSnapshot {
                measured_granularity_ms: 2,
                scheduler_interval_ms: 2,
                timer_resolution_requested: false,
                occlusion_workaround_enabled: false,
                high_resolution_waitable_timer: false,
            }
        );
    }

    #[test]
    fn unstable_samples_keep_extra_margin() {
        let snapshot = classify_samples(&[
            15_100, 15_200, 15_300, 15_400, 15_500, 15_600, 15_700, 16_400, 17_000,
        ]);
        assert_eq!(snapshot.measured_granularity_ms, 16);
        assert_eq!(snapshot.scheduler_interval_ms, 18);
    }

    #[test]
    fn waitable_timer_due_time_is_negative_100ns_units() {
        assert_eq!(relative_due_time(Duration::from_micros(250)), -2_500);
        assert_eq!(relative_due_time(Duration::from_millis(1)), -10_000);
    }
}
