# Archive

Reference code preserved for migration history. **Nothing here is compiled into the crate.**

## Files

### `reticulum_bridge.rs.legacy`

Pre-`rns-transport` TCP↔HDLC bridge. Connected to a standard `rnsd` over a raw TCP
socket using the legacy `rns-embedded-ffi` API (`RnsEmbeddedNode`,
`rns_embedded_node_push_inbound_wire`, `rns_embedded_node_take_outbound_wire`,
`rns_embedded_node_tick`).

**Replaced by:** Mode 3 in `src/node.rs` (`start_reticulum`), which uses
`rns_transport::transport::Transport` with `TcpClientInterface` for the same
purpose. The capability is not lost — only the implementation strategy changed.

**Why kept:** Useful as a reference if anyone needs to wire HDLC framing over a
raw TCP socket against the embedded-FFI API in the future. Otherwise pure
historical context — `git log` would tell the same story.

**Do not re-add to `lib.rs`** — the `rns-embedded-ffi` symbols it imports are no
longer part of the project's dependency graph; reactivating it requires
restoring that crate first.
