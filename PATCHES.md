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

## Received GOAWAY facts (Boundless W5d / #287)

Based on e9334a7, carry 00c0b0b533919d17fa8b4e1fefc67f1d86d342e9,
3b507022851e3d78ace13d0d43944d22340f78e2 and
05b454ced5b195dfc4ce772ec1e30e308c1b531c, adapted without the separate
W17c send-cap line (new commits d4effef, 68c2e62, 8436db7).
A received GOAWAY lost its last-stream-id at the error boundary, and
ensure_reason reduced it to a bare reason for poll_reset. Keep last_stream_id,
reason and debug bytes in the internal/public error and return that stored
GOAWAY whole to response/connection/new-request/readiness/poll_reset callers.
Do not infer processing, retry, or decode debug bytes as text; graceful
connection completion stays Ok. Preserve HeaderListTooLarge and Reset reason
handling and current header/trailer/exact-limit repairs. No aggregate
send-queue cap is included.

The accepted receive entry additionally fills a write-once GoAwayCapture before
stream-error fanout. Public ReceivedGoAway read-only accessors expose the exact
last_stream_id, reason and debug_data; Builder::received_go_away installs the
shared capture once before handshake. The snapshot survives a successful drain
without changing connection results or retry behavior. No external mutator is
exposed; only the native accepted-frame entry writes the capture.

Regression evidence on macOS arm64, code tip
1adb46958b777c5abc7d4a7bea84258ffc84aa4c:
`cargo test --workspace` exited 0; `cargo fmt --all --check` exited 0.
The workspace run includes:

- received_go_away_facts_reach_stream_and_connection_errors
- received_go_away_facts_reach_new_requests_and_poll_ready
- received_graceful_go_away_facts_reach_new_requests
- ensure_reason_returns_a_received_go_away_whole
- received_go_away_facts_reach_poll_reset
- recv_too_big_trailers_fails_the_stream
- server_recv_too_big_trailers_fails_the_stream
- header_list_size_admits_a_block_at_the_limit_and_names_one_over_it
- ensure_reason_keeps_header_limit_and_reset_reasons
- graceful_goaway_capture_survives_successful_drain
- eof_does_not_fill_goaway_capture
- first_accepted_frame_cannot_be_replaced

The existing 14 ignored tests remain ignored:
hpack::encoder::test::test_evicted_overflow; request_with_h1_version;
read_frame_too_big; read_headers_empty_payload; read_headers_with_pseudo;
read_headers_without_pseudo; expand_window_calls_are_coalesced;
expand_window_sends_window_update; recv_window_update_causes_overflow;
stream_close_by_recv_reset_frame_releases_capacity;
recv_push_promise_with_wrong_authority_is_stream_error;
accept_with_pending_connections_after_socket_close; recv_trailers_without_eos;
and the frame::unpack_octets_4 doctest.

Baseline compiler evidence (macOS arm64, pinned Rust toolchain, work tip
db341210890d8e95381c13b49b8356c43da552a4):
`build-slot cargo clippy -p h2 -p h2-tests --lib --tests --locked -- -D warnings` exited 101.
Eight diagnostics concern unchanged baseline code in src/hpack/table.rs
(309, 324, 356), src/proto/streams/prioritize.rs (717),
src/proto/streams/recv.rs (167), src/proto/streams/store.rs (90),
src/proto/streams/stream.rs (345), and src/frame/headers.rs (970).
Comparison with base e9334a79 confirms the diagnosed production expressions
were not introduced by W5d. The landing owner excludes these baseline results
from this change's gates; no unrelated repair or lint waiver is included.
The workspace regression and formatting results above remain unchanged.
