use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use hidapi::{HidApi, HidDevice};

pub trait Transport: Send {
    fn write(&self, data: &[u8]) -> Result<()>;
    fn read(&self, timeout: Duration) -> Result<Option<Vec<u8>>>;
}

pub struct HidTransport {
    device: Arc<Mutex<HidDevice>>,
}

impl HidTransport {
    pub fn open(vid: u16, pid: u16) -> Result<Self> {
        let api = HidApi::new()?;
        let device = api.open(vid, pid)?;
        Ok(Self {
            device: Arc::new(Mutex::new(device)),
        })
    }
}

impl Transport for HidTransport {
    fn write(&self, data: &[u8]) -> Result<()> {
        let device = self
            .device
            .lock()
            .map_err(|_| anyhow!("hid device lock poisoned"))?;
        device.write(data)?;
        Ok(())
    }

    fn read(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let device = self
            .device
            .lock()
            .map_err(|_| anyhow!("hid device lock poisoned"))?;
        let mut buffer = vec![0_u8; 320];
        let bytes = device.read_timeout(
            &mut buffer,
            timeout.as_millis().clamp(0, i32::MAX as u128) as i32,
        )?;
        if bytes == 0 {
            return Ok(None);
        }
        buffer.truncate(bytes);
        Ok(Some(buffer))
    }
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
}
