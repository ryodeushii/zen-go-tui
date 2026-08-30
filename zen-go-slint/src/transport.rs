use anyhow::Result;
use zen_go_tui::transport::{HidTransport, MockTransport, ThreadedTransport, Transport};

pub const ZEN_GO_VID: u16 = 0x23e5;
pub const ZEN_GO_PID: u16 = 0xa015;

pub fn open_transport(mock: bool) -> Result<Box<dyn Transport>> {
    if mock {
        return Ok(Box::new(MockTransport::default()));
    }

    let hid = HidTransport::open(ZEN_GO_VID, ZEN_GO_PID)?;
    Ok(Box::new(ThreadedTransport::spawn(hid)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_transport_opens_without_hid() {
        let transport = open_transport(true).expect("mock transport opens");

        assert!(transport.is_available().expect("mock transport available"));
    }
}
