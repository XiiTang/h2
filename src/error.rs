use crate::codec::{SendError, UserError};
use crate::frame::StreamId;
use crate::proto::{self, Initiator};

use bytes::Bytes;
use std::{error, fmt, io};

pub use crate::frame::Reason;

/// Represents HTTP/2 operation errors.
///
/// `Error` covers error cases raised by protocol errors caused by the
/// peer, I/O (transport) errors, and errors caused by the user of the library.
///
/// If the error was caused by the remote peer, then it will contain a
/// [`Reason`] which can be obtained with the [`reason`] function.
///
/// Cloning preserves protocol facts. For I/O errors it preserves the kind and
/// displayed message, without retaining the inner source chain.
///
/// [`Reason`]: struct.Reason.html
/// [`reason`]: #method.reason
#[derive(Debug)]
pub struct Error {
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    /// A RST_STREAM frame was received or sent.
    #[allow(dead_code)]
    Reset(StreamId, Reason, Initiator),

    /// A GO_AWAY frame was received or sent. The last field is the last
    /// stream identifier of a GO_AWAY frame received from the peer.
    GoAway(Bytes, Reason, Initiator, Option<StreamId>),

    /// A received header block exceeded SETTINGS_MAX_HEADER_LIST_SIZE, and
    /// the library reset the stream with `PROTOCOL_ERROR`.
    #[allow(dead_code)]
    HeaderListTooLarge(StreamId),

    /// The user created an error from a bare Reason.
    Reason(Reason),

    /// An error resulting from an invalid action taken by the user of this
    /// library.
    User(UserError),

    /// An `io::Error` occurred while trying to read or write.
    Io(io::Error),
}

impl Clone for Error {
    fn clone(&self) -> Self {
        Self {
            kind: match &self.kind {
                Kind::Reset(id, reason, initiator) => Kind::Reset(*id, *reason, *initiator),
                Kind::GoAway(data, reason, initiator, last) => {
                    Kind::GoAway(data.clone(), *reason, *initiator, *last)
                }
                Kind::HeaderListTooLarge(id) => Kind::HeaderListTooLarge(*id),
                Kind::Reason(reason) => Kind::Reason(*reason),
                Kind::User(error) => Kind::User(match error {
                    UserError::InactiveStreamId => UserError::InactiveStreamId,
                    UserError::UnexpectedFrameType => UserError::UnexpectedFrameType,
                    UserError::PayloadTooBig => UserError::PayloadTooBig,
                    UserError::Rejected => UserError::Rejected,
                    UserError::ReleaseCapacityTooBig => UserError::ReleaseCapacityTooBig,
                    UserError::OverflowedStreamId => UserError::OverflowedStreamId,
                    UserError::MalformedHeaders => UserError::MalformedHeaders,
                    UserError::MissingUriSchemeAndAuthority => {
                        UserError::MissingUriSchemeAndAuthority
                    }
                    UserError::PollResetAfterSendResponse => UserError::PollResetAfterSendResponse,
                    UserError::SendPingWhilePending => UserError::SendPingWhilePending,
                    UserError::SendSettingsWhilePending => UserError::SendSettingsWhilePending,
                    UserError::PeerDisabledServerPush => UserError::PeerDisabledServerPush,
                    UserError::InvalidInformationalStatusCode => {
                        UserError::InvalidInformationalStatusCode
                    }
                }),
                Kind::Io(error) => Kind::Io(io::Error::new(error.kind(), error.to_string())),
            },
        }
    }
}

// ===== impl Error =====

impl Error {
    /// If the error was caused by the remote peer, the error reason.
    ///
    /// This is either an error received by the peer or caused by an invalid
    /// action taken by the peer (i.e. a protocol error).
    pub fn reason(&self) -> Option<Reason> {
        match self.kind {
            Kind::Reset(_, reason, _) | Kind::GoAway(_, reason, _, _) | Kind::Reason(reason) => {
                Some(reason)
            }
            Kind::HeaderListTooLarge(_) => Some(Reason::PROTOCOL_ERROR),
            _ => None,
        }
    }

    /// Returns true if the error is an io::Error
    pub fn is_io(&self) -> bool {
        matches!(self.kind, Kind::Io(..))
    }

    /// Returns the error if the error is an io::Error
    pub fn get_io(&self) -> Option<&io::Error> {
        match self.kind {
            Kind::Io(ref e) => Some(e),
            _ => None,
        }
    }

    /// Returns the error if the error is an io::Error
    pub fn into_io(self) -> Option<io::Error> {
        match self.kind {
            Kind::Io(e) => Some(e),
            _ => None,
        }
    }

    pub(crate) fn from_io(err: io::Error) -> Self {
        Error {
            kind: Kind::Io(err),
        }
    }

    /// Returns true if the error is from a `GOAWAY`.
    pub fn is_go_away(&self) -> bool {
        matches!(self.kind, Kind::GoAway(..))
    }

    /// If the error is from a `GOAWAY` frame received from the peer, the
    /// frame's last stream identifier (RFC 9113 §6.8).
    ///
    /// Returns `None` for any other error, including a `GOAWAY` sent by this
    /// endpoint.
    pub fn last_stream_id(&self) -> Option<crate::StreamId> {
        match self.kind {
            Kind::GoAway(_, _, Initiator::Remote, Some(id)) => {
                Some(crate::StreamId::from_internal(id))
            }
            _ => None,
        }
    }

    /// If the error is from a `GOAWAY`, the frame's additional debug data,
    /// which is empty when the frame carried none.
    pub fn debug_data(&self) -> Option<&Bytes> {
        match self.kind {
            Kind::GoAway(ref debug_data, ..) => Some(debug_data),
            _ => None,
        }
    }

    /// Returns true if the error is from a `RST_STREAM`.
    pub fn is_reset(&self) -> bool {
        matches!(self.kind, Kind::Reset(..) | Kind::HeaderListTooLarge(_))
    }

    /// Returns true if a header block received on the stream, its initial
    /// headers or its trailers, exceeded the local
    /// SETTINGS_MAX_HEADER_LIST_SIZE ([`client::Builder::max_header_list_size`],
    /// [`server::Builder::max_header_list_size`]).
    ///
    /// The block was not delivered. The library reset the stream with
    /// `PROTOCOL_ERROR`, so the error is also a library reset with that
    /// reason.
    ///
    /// [`client::Builder::max_header_list_size`]: crate::client::Builder::max_header_list_size
    /// [`server::Builder::max_header_list_size`]: crate::server::Builder::max_header_list_size
    pub fn is_header_list_too_large(&self) -> bool {
        matches!(self.kind, Kind::HeaderListTooLarge(_))
    }

    /// Returns true if the error was received in a frame from the remote.
    ///
    /// Such as from a received `RST_STREAM` or `GOAWAY` frame.
    pub fn is_remote(&self) -> bool {
        matches!(
            self.kind,
            Kind::GoAway(_, _, Initiator::Remote, _) | Kind::Reset(_, _, Initiator::Remote)
        )
    }

    /// Returns true if the error was created by `h2`.
    ///
    /// Such as noticing some protocol error and sending a GOAWAY or RST_STREAM.
    pub fn is_library(&self) -> bool {
        matches!(
            self.kind,
            Kind::GoAway(_, _, Initiator::Library, _)
                | Kind::Reset(_, _, Initiator::Library)
                | Kind::HeaderListTooLarge(_)
        )
    }
}

impl From<proto::Error> for Error {
    fn from(src: proto::Error) -> Error {
        use crate::proto::Error::*;

        Error {
            kind: match src {
                Reset(stream_id, reason, initiator) => Kind::Reset(stream_id, reason, initiator),
                GoAway(debug_data, reason, initiator, last_stream_id) => {
                    Kind::GoAway(debug_data, reason, initiator, last_stream_id)
                }
                HeaderListTooLarge(stream_id) => Kind::HeaderListTooLarge(stream_id),
                Io(kind, inner) => {
                    Kind::Io(inner.map_or_else(|| kind.into(), |inner| io::Error::new(kind, inner)))
                }
            },
        }
    }
}

impl From<Reason> for Error {
    fn from(src: Reason) -> Error {
        Error {
            kind: Kind::Reason(src),
        }
    }
}

impl From<SendError> for Error {
    fn from(src: SendError) -> Error {
        match src {
            SendError::User(e) => e.into(),
            SendError::Connection(e) => e.into(),
        }
    }
}

impl From<UserError> for Error {
    fn from(src: UserError) -> Error {
        Error {
            kind: Kind::User(src),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let debug_data = match self.kind {
            Kind::Reset(_, reason, Initiator::User) => {
                return write!(fmt, "stream error sent by user: {}", reason)
            }
            Kind::Reset(_, reason, Initiator::Library) => {
                return write!(fmt, "stream error detected: {}", reason)
            }
            Kind::HeaderListTooLarge(_) => {
                return write!(fmt, "stream error detected: {}", Reason::PROTOCOL_ERROR)
            }
            Kind::Reset(_, reason, Initiator::Remote) => {
                return write!(fmt, "stream error received: {}", reason)
            }
            Kind::GoAway(ref debug_data, reason, Initiator::User, _) => {
                write!(fmt, "connection error sent by user: {}", reason)?;
                debug_data
            }
            Kind::GoAway(ref debug_data, reason, Initiator::Library, _) => {
                write!(fmt, "connection error detected: {}", reason)?;
                debug_data
            }
            Kind::GoAway(ref debug_data, reason, Initiator::Remote, _) => {
                write!(fmt, "connection error received: {}", reason)?;
                debug_data
            }
            Kind::Reason(reason) => return write!(fmt, "protocol error: {}", reason),
            Kind::User(ref e) => return write!(fmt, "user error: {}", e),
            Kind::Io(ref e) => return e.fmt(fmt),
        };

        if !debug_data.is_empty() {
            write!(fmt, " ({:?})", debug_data)?;
        }

        Ok(())
    }
}

impl error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Reason;

    #[test]
    fn error_from_reason() {
        let err = Error::from(Reason::HTTP_1_1_REQUIRED);
        assert_eq!(err.reason(), Some(Reason::HTTP_1_1_REQUIRED));
    }
    #[test]
    fn clone_retains_protocol_facts_and_io_kind_and_message() {
        let mut errors = vec![
            Error {
                kind: Kind::Reason(Reason::CANCEL),
            },
            Error {
                kind: Kind::HeaderListTooLarge(StreamId::from(7)),
            },
            Error::from_io(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "fixture reset",
            )),
        ];
        for initiator in [Initiator::User, Initiator::Library, Initiator::Remote] {
            errors.push(Error {
                kind: Kind::GoAway(
                    Bytes::from_static(b"\x00\xffgoaway"),
                    Reason::ENHANCE_YOUR_CALM,
                    initiator,
                    Some(StreamId::from(3)),
                ),
            });
            errors.push(Error {
                kind: Kind::Reset(StreamId::from(5), Reason::REFUSED_STREAM, initiator),
            });
        }
        for error in [
            UserError::InactiveStreamId,
            UserError::UnexpectedFrameType,
            UserError::PayloadTooBig,
            UserError::Rejected,
            UserError::ReleaseCapacityTooBig,
            UserError::OverflowedStreamId,
            UserError::MalformedHeaders,
            UserError::MissingUriSchemeAndAuthority,
            UserError::PollResetAfterSendResponse,
            UserError::SendPingWhilePending,
            UserError::SendSettingsWhilePending,
            UserError::PeerDisabledServerPush,
            UserError::InvalidInformationalStatusCode,
        ] {
            errors.push(Error::from(error));
        }
        for original in errors {
            let cloned = original.clone();
            assert_eq!(cloned.reason(), original.reason());
            assert_eq!(cloned.debug_data(), original.debug_data());
            assert_eq!(cloned.last_stream_id(), original.last_stream_id());
            assert_eq!(cloned.is_remote(), original.is_remote());
            assert_eq!(cloned.is_library(), original.is_library());
            assert_eq!(cloned.is_reset(), original.is_reset());
            assert_eq!(
                cloned.is_header_list_too_large(),
                original.is_header_list_too_large()
            );
            assert_eq!(cloned.to_string(), original.to_string());
            match (&original.kind, &cloned.kind) {
                (Kind::GoAway(a, ar, ai, al), Kind::GoAway(b, br, bi, bl)) => {
                    assert_eq!((a, ar, ai, al), (b, br, bi, bl));
                }
                (Kind::Reset(a, ar, ai), Kind::Reset(b, br, bi)) => {
                    assert_eq!((a, ar, ai), (b, br, bi));
                }
                (Kind::HeaderListTooLarge(a), Kind::HeaderListTooLarge(b)) => assert_eq!(a, b),
                (Kind::Reason(a), Kind::Reason(b)) => assert_eq!(a, b),
                (Kind::User(a), Kind::User(b)) => assert_eq!(format!("{a:?}"), format!("{b:?}")),
                (Kind::Io(a), Kind::Io(b)) => assert_eq!(a.kind(), b.kind()),
                _ => panic!("cloning changed the error kind"),
            }
        }
    }
}
