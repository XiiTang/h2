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
