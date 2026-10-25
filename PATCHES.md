# Boundless HTTP/2 patches

Based on upstream v0.4.19 (d57d1b8). Keep upstream history and licenses.

- #951 (d4a37fcac2c382746b6ff3509d8bd5871235ab37): reject GOAWAY with a
  non-zero frame stream ID.
- #956 (e43124bb5579bae0306d4a8f70dc127a640534c4): clear streams when
  transitioning to connection shutdown, including the close race.
- #959 (3c5f61c9bac3b9ab9b24031a3bb90d8b58eccb9a): reject missing :status
  with a stream protocol error while allowing unrelated streams to continue.

The receive-buffer overflow unit fixture explicitly unlinks/removes its stream
before Store teardown, as required by #956's empty-slab assertion. Its overflow
assertions remain intact. `cargo test --workspace` passes on macOS arm64; existing
ignored checks are reported separately. No business retry is added.

## Oversized trailers fail the stream (Boundless #359)

`HeaderBlock::load` stops keeping fields once a block reaches the local
SETTINGS_MAX_HEADER_LIST_SIZE and marks the frame oversize
(src/frame/headers.rs:882-906 at 7b547e0). `Recv::recv_headers` refuses an
oversized initial block (src/proto/streams/recv.rs:213-240): a server answers
431 when it still can, and otherwise the stream is reset with a library
PROTOCOL_ERROR, which the application sees as that stream's error.
`Recv::recv_trailers` (src/proto/streams/recv.rs:419-441) never read the mark,
so a trailer block over the limit reached the application truncated, as if it
were complete; upstream master 3c5f61c is the same (recv.rs:409). A gRPC
`grpc-status` after a large field was lost with it.

`Recv::recv_trailers`, the one place received trailers are delivered (its single
caller is `Inner::recv_headers`, src/proto/streams/streams.rs:581), now fails an
oversized block with `Error::library_reset(id, PROTOCOL_ERROR)`, as
`recv_headers` fails an oversized initial block that can no longer be a 431. The
error goes through `reset_on_recv_stream_err`: RST_STREAM(PROTOCOL_ERROR) is sent
when the stream is still open locally, and none when the trailers' END_STREAM
closed it, as for an oversized initial block with END_STREAM. `poll_trailers`
returns that error (`is_reset`, `is_library`, `reason() == PROTOCOL_ERROR`);
hyper hands it on as `Error::new_h2` (hyper e158e802 src/body/incoming.rs:292-305)
and tonic maps PROTOCOL_ERROR to `Internal` (tonic 0.14.6 src/status.rs:393-404).
Blocks over four times the limit still end the connection with
ENHANCE_YOUR_CALM, as before. No API is added.

Regression (tests/h2-tests/tests/trailers.rs):
`recv_too_big_trailers_fails_the_stream` sets a 64-byte client limit and
receives trailers `grpc-status: 0` and a 100-byte `x-big` on a stream whose
request ended (no reset sent) and on one whose request is open
(RST_STREAM(3, PROTOCOL_ERROR) sent); both `poll_trailers` calls fail with the
library PROTOCOL_ERROR reset. `server_recv_too_big_trailers_fails_the_stream` is
the server's case with a 200-byte limit: the request's trailers fail and the
server sends RST_STREAM(1, PROTOCOL_ERROR). At 7b547e0 both fail:
`poll_trailers` returns `Some({"grpc-status": "0"})` and `Some({"x-small": "0"})`.
