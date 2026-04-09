use std::collections::VecDeque;
use std::error::Error as StdError;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use hidapi::{HidApi, HidDevice};

pub trait Transport: Send {
    fn write(&self, data: &[u8]) -> Result<()>;
    fn read(&self, timeout: Duration) -> Result<Option<Vec<u8>>>;

    fn is_available(&self) -> Result<bool> {
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    DeviceUnavailable,
    DeviceDisconnected,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceUnavailable => write!(f, "device unavailable"),
            Self::DeviceDisconnected => write!(f, "device disconnected"),
        }
    }
}

impl StdError for TransportError {}

pub fn is_device_error(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<TransportError>().is_some())
}

const HID_RECONNECT_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Default)]
struct HidTransportState {
    device: Option<HidDevice>,
    last_open_attempt: Option<Instant>,
    read_buffer: Vec<u8>,
}

pub struct HidTransport {
    vid: u16,
    pid: u16,
    state: Arc<Mutex<HidTransportState>>,
}

impl HidTransport {
    pub fn open(vid: u16, pid: u16) -> Result<Self> {
        let device = open_hid_device(vid, pid)?;
        Ok(Self {
            vid,
            pid,
            state: Arc::new(Mutex::new(HidTransportState {
                device: Some(device),
                last_open_attempt: None,
                read_buffer: vec![0_u8; 320],
            })),
        })
    }

    fn ensure_device(state: &mut HidTransportState, vid: u16, pid: u16) -> Result<bool> {
        if state.device.is_some() {
            return Ok(true);
        }

        if state
            .last_open_attempt
            .is_some_and(|instant| instant.elapsed() < HID_RECONNECT_INTERVAL)
        {
            return Ok(false);
        }

        state.last_open_attempt = Some(Instant::now());

        match open_hid_device(vid, pid) {
            Ok(device) => {
                state.device = Some(device);
                state.last_open_attempt = None;
                Ok(true)
            }
            Err(error) if is_device_error(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

impl Transport for HidTransport {
    fn write(&self, data: &[u8]) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("hid device lock poisoned"))?;

        if !Self::ensure_device(&mut state, self.vid, self.pid)? {
            return Err(TransportError::DeviceUnavailable.into());
        }

        let Some(device) = state.device.as_ref() else {
            return Err(TransportError::DeviceUnavailable.into());
        };

        if device.write(data).is_err() {
            state.device = None;
            state.last_open_attempt = Some(Instant::now());
            return Err(TransportError::DeviceDisconnected.into());
        }
        Ok(())
    }

    fn read(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("hid device lock poisoned"))?;

        if !Self::ensure_device(&mut state, self.vid, self.pid)? {
            return Ok(None);
        }

        let mut buffer = std::mem::take(&mut state.read_buffer);
        let Some(device) = state.device.as_ref() else {
            state.read_buffer = buffer;
            return Ok(None);
        };

        let bytes = match device.read_timeout(
            &mut buffer,
            timeout.as_millis().clamp(0, i32::MAX as u128) as i32,
        ) {
            Ok(bytes) => bytes,
            Err(_) => {
                state.device = None;
                state.last_open_attempt = Some(Instant::now());
                state.read_buffer = buffer;
                return Err(TransportError::DeviceDisconnected.into());
            }
        };
        if bytes == 0 {
            state.read_buffer = buffer;
            return Ok(None);
        }
        let result = buffer[..bytes].to_vec();
        state.read_buffer = buffer;
        Ok(Some(result))
    }

    fn is_available(&self) -> Result<bool> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("hid device lock poisoned"))?;
        Self::ensure_device(&mut state, self.vid, self.pid)
    }
}

fn open_hid_device(vid: u16, pid: u16) -> Result<HidDevice> {
    let api = HidApi::new()?;
    let candidates: Vec<_> = api
        .device_list()
        .filter(|device| device.vendor_id() == vid && device.product_id() == pid)
        .collect();

    if candidates.is_empty() {
        return Err(anyhow!(TransportError::DeviceUnavailable).context(format!(
            "no HID interfaces found for {:04x}:{:04x}",
            vid, pid
        )));
    }

    let mut failures = Vec::new();
    for candidate in candidates {
        match candidate.open_device(&api) {
            Ok(device) => return Ok(device),
            Err(error) => failures.push(format!(
                "path={} iface={} usage_page=0x{:04x} usage=0x{:04x} product={}: {}",
                candidate.path().to_string_lossy(),
                candidate.interface_number(),
                candidate.usage_page(),
                candidate.usage(),
                candidate.product_string().unwrap_or("?"),
                error
            )),
        }
    }

    Err(anyhow!(TransportError::DeviceUnavailable).context(format!(
        "found {} matching HID interface(s) for {:04x}:{:04x}, but none opened:\n{}",
        failures.len(),
        vid,
        pid,
        failures.join("\n")
    )))
}

#[derive(Clone, Default)]
pub struct MockTransport {
    inner: Arc<Mutex<MockTransportInner>>,
}

#[derive(Default)]
struct MockTransportInner {
    reads: VecDeque<Vec<u8>>,
    writes: Vec<Vec<u8>>,
}

impl MockTransport {
    pub fn push_read(&self, data: Vec<u8>) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.reads.push_back(data);
        }
    }

    pub fn take_writes(&self) -> Vec<Vec<u8>> {
        if let Ok(mut inner) = self.inner.lock() {
            return std::mem::take(&mut inner.writes);
        }
        Vec::new()
    }
}

impl Transport for MockTransport {
    fn write(&self, data: &[u8]) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow!("mock transport lock poisoned"))?;
        inner.writes.push(data.to_vec());
        Ok(())
    }

    fn read(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow!("mock transport lock poisoned"))?;
        Ok(inner.reads.pop_front())
    }

    fn is_available(&self) -> Result<bool> {
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_transport_records_writes_and_reads_frames() {
        let transport = MockTransport::default();
        transport.push_read(vec![1, 2, 3]);
        transport.write(&[9, 8, 7]).expect("write");

        let read = transport.read(Duration::from_millis(10)).expect("read");
        assert_eq!(read, Some(vec![1, 2, 3]));
        assert_eq!(transport.take_writes(), vec![vec![9, 8, 7]]);
    }

    #[test]
    fn is_device_error_matches_context_wrapped_transport_error() {
        let error = anyhow!(TransportError::DeviceUnavailable).context("wrapped device error");

        assert!(is_device_error(&error));
    }
}
