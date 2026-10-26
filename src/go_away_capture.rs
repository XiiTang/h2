//! Write-once facts of an accepted peer GOAWAY, independent of drive completion.
use crate::{frame, Reason, StreamId};
use bytes::Bytes;
use std::sync::{Arc, Mutex};

/// Facts of the first accepted GOAWAY received from the peer.
#[derive(Clone, Debug)]
pub struct ReceivedGoAway {
    last_stream_id: StreamId,
    reason: Reason,
    debug_data: Bytes,
}
impl ReceivedGoAway {
    /// The peer's last stream identifier.
    pub fn last_stream_id(&self) -> StreamId {
        self.last_stream_id
    }
    /// The peer's reason code.
    pub fn reason(&self) -> Reason {
        self.reason
    }
    /// The exact additional debug bytes, including an empty payload.
    pub fn debug_data(&self) -> &[u8] {
        &self.debug_data
    }
}

/// Shared snapshot retained even when the connection drains successfully.
/// Only the native accepted-frame receive path can fill it.
#[derive(Clone, Debug, Default)]
pub struct GoAwayCapture {
    received: Arc<Mutex<Option<ReceivedGoAway>>>,
}
impl GoAwayCapture {
    /// The first accepted peer GOAWAY, if one has been received.
    pub fn received(&self) -> Option<ReceivedGoAway> {
        self.received.lock().unwrap().clone()
    }
    pub(crate) fn retain(&self, frame: &frame::GoAway) {
        let mut received = self.received.lock().unwrap();
        if received.is_none() {
            *received = Some(ReceivedGoAway {
                last_stream_id: StreamId::from_internal(frame.last_stream_id()),
                reason: frame.reason(),
                debug_data: frame.debug_data().clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_accepted_frame_cannot_be_replaced() {
        let capture = GoAwayCapture::default();
        capture.retain(&frame::GoAway::with_debug_data(
            1.into(),
            Reason::NO_ERROR,
            Bytes::from_static(b"\x00\xffdrain"),
        ));
        capture.retain(&frame::GoAway::with_debug_data(
            0.into(),
            Reason::INTERNAL_ERROR,
            Bytes::from_static(b"later"),
        ));
        let received = capture.received().unwrap();
        assert_eq!(received.last_stream_id().as_u32(), 1);
        assert_eq!(received.reason(), Reason::NO_ERROR);
        assert_eq!(received.debug_data(), b"\x00\xffdrain");
    }
}
