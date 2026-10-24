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

## Oversized header blocks: trailers fail the stream, the error names the limit (Boundless #359)

`HeaderBlock::load` stops keeping fields once a block reaches the local
SETTINGS_MAX_HEADER_LIST_SIZE and marks the frame oversize
(src/frame/headers.rs:882-906 at 7b547e0). `Recv::recv_headers` refuses an
oversized initial block (src/proto/streams/recv.rs:213-240): a server answers
431 when it still can, and otherwise the stream is reset with a library
PROTOCOL_ERROR, which the application sees as that stream's error.
`Recv::recv_trailers` (src/proto/streams/recv.rs:419-441) never read the mark,
so a trailer block over the limit reached the application truncated, as if it
were complete; upstream master 3c5f61c is the same (recv.rs:409). A gRPC
`grpc-status` after a large field was lost with it. Three more facts:

- The application could not tell a header list refusal from any other library
  PROTOCOL_ERROR reset (malformed headers, a content-length never met): both
  are `Kind::Reset(_, PROTOCOL_ERROR, Library)` (src/error.rs:25-43), so an
  HTTP or gRPC client could not report which configured limit the peer's
  block exceeded.
- The block was marked oversize when its size reached the limit
  (`headers_size >= max_header_list_size`, src/frame/headers.rs:882). RFC 9113
  §6.5.2 defines the setting as the maximum field section size the endpoint
  is prepared to accept, so a block of exactly that size was refused against
  the RFC; Envoy, for one, refuses only a larger block
  (source/common/http/http2/codec_impl.cc:1748).
- The same mark is set when the field map's capacity is exceeded
  (src/frame/headers.rs:949-956), which is not the header list size.

`recv_trailers`, the one place received trailers are delivered (its single
caller is `Inner::recv_headers`, src/proto/streams/streams.rs:581), now fails an
oversized block as `recv_headers` fails an oversized initial block that can no
longer be a 431. Both go through one private `recv::over_size`, which names a
block over the header list size `proto::Error::HeaderListTooLarge(id)` and
leaves one over size only because its field map is full a plain library reset.
`reset_on_recv_stream_err`, the path every received-headers stream error takes,
resets the stream as for a library PROTOCOL_ERROR (RST_STREAM sent while the
stream is still open locally, none when the block's END_STREAM closed it,
counted against the local error resets) and keeps the cause as the stream's
end. The new `Error::is_header_list_too_large()` is true for that end, on the
response future, `poll_trailers`/`poll_data` and the server's request body.
The error stays a library reset: `is_reset()`, `is_library()`,
`reason() == Some(PROTOCOL_ERROR)` and `Display` are unchanged, and
`poll_reset` still returns `Ok(PROTOCOL_ERROR)`, so hyper (`Error::new_h2`,
hyper e158e802 src/body/incoming.rs:292-305) and tonic (PROTOCOL_ERROR maps to
`Internal`, tonic 0.14.6 src/status.rs:393-404) behave as before. A block is
oversize when its size exceeds the limit. Blocks over four times the limit
still end the connection with ENHANCE_YOUR_CALM, a connection error that is
not the stream's, and an oversized PUSH_PROMISE still drops the promise the
application never sees; neither is named by the predicate.

Regression (tests/h2-tests/tests/trailers.rs):
`recv_too_big_trailers_fails_the_stream` sets a 64-byte client limit and
receives trailers `grpc-status: 0` and a 100-byte `x-big` on a stream whose
request ended (no reset sent) and on one whose request is open
(RST_STREAM(3, PROTOCOL_ERROR) sent); both `poll_trailers` calls fail with the
library PROTOCOL_ERROR reset and `is_header_list_too_large()`.
`server_recv_too_big_trailers_fails_the_stream` is the server's case with a
200-byte limit: the request's trailers fail the same way and the server sends
RST_STREAM(1, PROTOCOL_ERROR). At 7b547e0 both fail: `poll_trailers` returns
`Some({"grpc-status": "0"})` and `Some({"x-small": "0"})`.
`header_list_size_admits_a_block_at_the_limit_and_names_one_over_it`, under a
200-byte limit, receives response headers and trailers of exactly 200 bytes
(delivered whole), headers of 201 bytes (the response fails with the
predicate, RST_STREAM(3) sent), trailers of 201 bytes (they fail with the
predicate), and an ordinary PROTOCOL_ERROR reset from a content-length the
trailers never meet (the predicate is false). With `>=` restored the
200-byte headers are refused; with `over_size` returning a plain reset every
predicate assertion fails; before the patch the predicate does not exist.

## Received GOAWAY facts in errors (W5d)

Boundless reports a peer's GOAWAY as `Http.GoAway {last_stream_id, error_code,
debug_data}` (driver plan v23 W5d; RFC 9113 §6.8). Upstream keeps a received
GOAWAY's debug data and reason in `proto::Error::GoAway` (src/proto/error.rs:12,
:56) and the public `Kind::GoAway` (src/error.rs:25-43), but drops its last
stream identifier when it builds the error, for the streams it fails above that
identifier (src/proto/streams/streams.rs:800-816) and for the connection's own
result (src/proto/connection.rs:238-256); the public `Error` exposes only
`reason()`, so the debug data is reachable only through `Display` text.

The patch keeps the received frame's last stream identifier in both error
representations and adds `Error::last_stream_id() -> Option<StreamId>` (only for
a GOAWAY received from the peer) and `Error::debug_data() -> Option<&Bytes>`
(for any GOAWAY). Nothing is inferred from the facts; GOAWAY handling, stream
selection and `Display` are unchanged.

Regression: `received_go_away_facts_reach_stream_and_connection_errors`
(tests/h2-tests/tests/client_request.rs) receives GOAWAY(1, ENHANCE_YOUR_CALM,
"too_many_pings") with streams 1 and 3 open; stream 3's error and the
connection's error both carry the last stream id, reason and debug data. Before
the patch the test does not compile (neither accessor exists) and no public API
yields the last stream id.
