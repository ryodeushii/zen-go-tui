//! Timing and scheduling helpers for the device polling loop.

use std::time::{Duration, Instant};

/// Interval for the initial device connection retry.
pub(crate) const INITIAL_DEVICE_RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// Interval for backoff device retries after the first attempt.
pub(crate) const BACKOFF_DEVICE_RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Poll interval when the device is actively responding.
pub(crate) const ACTIVE_DEVICE_POLL_INTERVAL: Duration = Duration::from_millis(16);

/// Poll interval for the TUI when the device has been idle.
pub(crate) const IDLE_TUI_DEVICE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Poll interval for headless mode when the device has been idle.
pub(crate) const IDLE_HEADLESS_DEVICE_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How long after the last activity before switching to idle polling.
pub(crate) const DEVICE_POLL_BACKOFF_AFTER: Duration = Duration::from_secs(1);

/// Redraw interval when the UI is dirty.
pub(crate) const DIRTY_REDRAW_INTERVAL: Duration = Duration::from_millis(50);

/// Redraw interval when the UI is idle.
pub(crate) const IDLE_REDRAW_INTERVAL: Duration = Duration::from_secs(1);

/// Decide whether to draw a new frame based on elapsed time and dirty state.
pub(crate) fn should_draw_frame(
    last_draw_at: Option<Instant>,
    needs_redraw: bool,
    now: Instant,
) -> bool {
    let Some(last_draw_at) = last_draw_at else {
        return true;
    };

    let elapsed = now.duration_since(last_draw_at);
    (needs_redraw && elapsed >= DIRTY_REDRAW_INTERVAL) || elapsed >= IDLE_REDRAW_INTERVAL
}

/// Decide whether to probe for reconnection based on backoff timing.
pub(crate) fn should_probe_reconnect(
    last_probe_at: Option<Instant>,
    attempts: usize,
    now: Instant,
) -> bool {
    let Some(last_probe_at) = last_probe_at else {
        return true;
    };

    now.duration_since(last_probe_at) >= device_retry_interval(attempts.saturating_add(1))
}

/// Compute the retry interval for a given retry count (backoff after first).
pub(crate) fn device_retry_interval(retries: usize) -> Duration {
    if retries <= 1 {
        INITIAL_DEVICE_RETRY_INTERVAL
    } else {
        BACKOFF_DEVICE_RETRY_INTERVAL
    }
}

/// Compute the device poll interval based on recent activity and interactivity.
pub(crate) fn device_poll_interval(
    last_activity_at: Option<Instant>,
    interactive: bool,
    now: Instant,
) -> Duration {
    let Some(last_activity_at) = last_activity_at else {
        return ACTIVE_DEVICE_POLL_INTERVAL;
    };

    if now.duration_since(last_activity_at) < DEVICE_POLL_BACKOFF_AFTER {
        ACTIVE_DEVICE_POLL_INTERVAL
    } else if interactive {
        IDLE_TUI_DEVICE_POLL_INTERVAL
    } else {
        IDLE_HEADLESS_DEVICE_POLL_INTERVAL
    }
}
