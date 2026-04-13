# Reduce Allocations in Hot Path

## Discovery

### Original Request
- "proceed with next p2" — referring to Priority 2 task #2: "Reduce allocations in hot path"

### Current Allocation Hotspots

| Location | Allocation | Frequency | Impact |
|----------|-----------|-----------|--------|
| `frame.rs:100,105` | `bytes[..].to_vec()` per frame | 30fps × poll rate | Every frame parse allocates |
| `encoder.rs:143,325,359` | `vec![0_u8; 320]` per command | Every device write | Command encoding allocates |
| `frame.rs:61` | `bytes.to_vec()` in `parse()` wrapper | Every frame parse | Extra copy before parse_owned |
| `mod.rs:319` | `raw: Vec<u8>` stored in observe_frame | Every snapshot frame | Raw frame history accumulates |

### Proposed Changes

1. **`Frame` enum**: Change `raw: Vec<u8>` → `raw: [u8; 320]` (fixed-size array, no heap)
2. **`encode_command` / `encode_query`**: Return `[u8; 320]` instead of `Vec<u8>`
3. **`Frame::parse()`**: Take `&[u8]` directly, no intermediate `.to_vec()`
4. **`observe_frame`**: Accept `&[u8; 320]` instead of `Vec<u8>`

### Risk Assessment
- `Frame::raw` is used by raw view display, baseline comparison, query log — all need `[u8; 320]` compatibility
- Transport trait returns `Vec<u8>` — may need to change or add conversion
- All callers of `encode_command`, `encode_query` need updating

---

## Non-Goals
- No arena allocators or ring buffers (too invasive for this task)
- No changes to command queue internals
- No behavioral changes

---

## Tasks

### 1. Change `Frame` enum to use `[u8; 320]` instead of `Vec<u8>`

**Depends on**: none

**Files:**
- Modify: `antelope-protocol/src/frame.rs`
- Modify: `antelope-protocol/src/encoder.rs` (return `[u8; 320]`)
- Modify: `src/transport.rs` (adapt if needed)
- Modify: `src/app/mod.rs` (observe_frame, push_query_reply_log)
- Modify: `src/app/controller.rs` (poll_device, send paths)
- Test: `src/ui/tests.rs` (may need updates)

**What to do**:
1. In `frame.rs`, change `Frame` enum:
   ```rust
   pub enum Frame {
       Snapshot {
           snapshot: DeviceStateSnapshot,
           raw: [u8; 320],  // was Vec<u8>
       },
       QueryReply {
           reply: QueryResponse,
           raw: [u8; 320],  // was Vec<u8>
       },
       Auxiliary {
           bytes: [u8; 320],  // was Vec<u8>
       },
   }
   ```
2. Update `Frame::parse_owned` to copy into `[u8; 320]` instead of `.to_vec()`
3. Update `Frame::parse` to not call `.to_vec()` before `parse_owned`
4. In `encoder.rs`, change `encode_command`, `encode_query`, etc. to return `[u8; 320]` instead of `Vec<u8>`
5. Update all callers throughout the codebase
6. Verify:
   - Run: `cargo build`
   - Run: `cargo test`
7. Commit

**Verify**:
- [ ] `cargo build` → 0 errors
- [ ] `cargo test` → all pass

### 2. Update Transport trait and callers

**Depends on**: 1

**Files:**
- Modify: `src/transport.rs`
- Modify: `src/app/controller.rs`

**What to do**:
1. If `Transport::read()` returns `Vec<u8>`, add conversion to `[u8; 320]` at the call site
2. If `Transport::write()` takes `&[u8]`, no change needed (works with both)
3. Verify build and tests
4. Commit

**Verify**:
- [ ] `cargo build` → 0 errors
- [ ] `cargo test` → all pass

### 3. Update raw view and baseline storage

**Depends on**: 2

**Files:**
- Modify: `src/app/mod.rs` (RawViewState, observe_query_request, capture_raw_baseline)
- Modify: `src/app/controller.rs` if needed

**What to do**:
1. Change `RawViewState` fields from `Vec<u8>` to `[u8; 320]` where storing raw frames
2. Update `observe_query_request`, `capture_raw_baseline` to work with `[u8; 320]`
3. Verify build and tests
4. Commit

**Verify**:
- [ ] `cargo build` → 0 errors
- [ ] `cargo test` → all pass
