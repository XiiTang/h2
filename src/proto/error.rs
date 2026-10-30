use crate::codec::SendError;
use crate::frame::{self, Reason, StreamId};

use bytes::Bytes;
use std::fmt;
use std::io;
use std::sync::Arc;

/// Either an H2 reason  or an I/O error
#[derive(Clone, Debug)]
pub enum Error {
    Reset(StreamId, Reason, Initiator),
    /// Received GOAWAY retains the last stream identifier.
    GoAway(Bytes, Reason, Initiator, Option<StreamId>),
    /// The stream was reset with a library PROTOCOL_ERROR because a header
    /// block received on it exceeded SETTINGS_MAX_HEADER_LIST_SIZE.
    HeaderListTooLarge(StreamId),
    Io(Arc<io::Error>),
}

pub struct GoAway {
    pub debug_data: Bytes,
    pub reason: Reason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Initiator {
    User,
    Library,
    Remote,
}

impl Error {
    pub(crate) fn is_local(&self) -> bool {
        match *self {
            Self::Reset(_, _, initiator) | Self::GoAway(_, _, initiator, _) => initiator.is_local(),
            Self::HeaderListTooLarge(_) | Self::Io(..) => true,
        }
    }

    pub(crate) fn user_go_away(reason: Reason) -> Self {
        Self::GoAway(Bytes::new(), reason, Initiator::User, None)
    }

    pub(crate) fn library_reset(stream_id: StreamId, reason: Reason) -> Self {
        Self::Reset(stream_id, reason, Initiator::Library)
    }

    pub(crate) fn header_list_too_large(stream_id: StreamId) -> Self {
        Self::HeaderListTooLarge(stream_id)
    }

    pub(crate) fn library_go_away(reason: Reason) -> Self {
        Self::GoAway(Bytes::new(), reason, Initiator::Library, None)
    }

    pub(crate) fn library_go_away_data(reason: Reason, debug_data: impl Into<Bytes>) -> Self {
        Self::GoAway(debug_data.into(), reason, Initiator::Library, None)
    }

    pub(crate) fn remote_reset(stream_id: StreamId, reason: Reason) -> Self {
        Self::Reset(stream_id, reason, Initiator::Remote)
    }

    pub(crate) fn remote_go_away(frame: &frame::GoAway) -> Self {
        Self::GoAway(
            frame.debug_data().clone(),
            frame.reason(),
            Initiator::Remote,
            Some(frame.last_stream_id()),
        )
    }
}

impl Initiator {
    fn is_local(&self) -> bool {
        match *self {
            Self::User | Self::Library => true,
            Self::Remote => false,
        }
    }

    pub(crate) fn is_library(&self) -> bool {
        matches!(self, Self::Library)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Self::Reset(_, reason, _) | Self::GoAway(_, reason, _, _) => reason.fmt(fmt),
            Self::HeaderListTooLarge(_) => Reason::PROTOCOL_ERROR.fmt(fmt),
            Self::Io(ref error) => error.fmt(fmt),
        }
    }
}

impl From<io::ErrorKind> for Error {
    fn from(src: io::ErrorKind) -> Self {
        io::Error::from(src).into()
    }
}

impl From<io::Error> for Error {
    fn from(src: io::Error) -> Self {
        Error::Io(Arc::new(src))
    }
}

impl From<Error> for SendError {
    fn from(src: Error) -> Self {
        Self::Connection(src)
    }
}
