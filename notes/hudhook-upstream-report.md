# 给 veeenu/hudhook 的 bug 报告（可直接粘贴）

> 目标仓库：https://github.com/veeenu/hudhook
> 建议开一个 issue，或者直接开三个小 PR（三处互相独立，分开更容易 review）。
> 三处都基于 hudhook 0.8.2 的 `src/renderer/backend/dx12.rs` 和 `src/util.rs`。

---

**Title:** Three D3D12 backend correctness bugs (fence never waited, illegal
state transition on texture re-upload, missing cross-queue ownership transfer)

Found these while debugging texture corruption in a game overlay built on
hudhook 0.8.2. All three are independent of my application and independent of
each other. Happy to split into separate PRs.

---

## 1. `Fence` starts at 0, so the first submission on every fence is never waited for

`src/util.rs`

```rust
let fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }?;
let value = AtomicU64::new(0);
```

Every call site follows this order:

```rust
queue.Signal(fence.fence(), fence.value())?;   // Signal(0)
fence.wait()?;
fence.incr();
```

and `wait()` is:

```rust
if self.fence.GetCompletedValue() < value {   // 0 < 0  -> false
    ...
}
```

On the first call `value` is 0 and the fence was created at 0, so
`GetCompletedValue() < value` is false and **`wait()` returns immediately
without waiting for anything**. Signalling a fence with a value it already has
is also a no-op. Every subsequent call is fine; only the first one on each
fence is unsynchronised.

That affects `TextureHeap`'s fence (first `load_texture`) and the render
engine's fence (first `render`).

**Fix:** start the counter at 1.

```rust
let value = AtomicU64::new(1);
```

---

## 2. `upload_texture` emits an illegal state transition on every re-upload

`src/renderer/backend/dx12.rs`

`Texture` does not record what state the resource was left in:

```rust
struct Texture {
    resource: ID3D12Resource,
    gpu_desc: D3D12_GPU_DESCRIPTOR_HANDLE,
    width: u32,
    height: u32,
}
```

`create_texture` creates it in `COPY_DEST`, and `upload_texture` unconditionally
ends with:

```rust
let barriers = [util::create_barrier(
    &texture.resource,
    D3D12_RESOURCE_STATE_COPY_DEST,
    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
)];
```

Correct the first time. On any later call — i.e. through `replace_texture` —
the resource is already in `PIXEL_SHADER_RESOURCE`, so:

- `CopyTextureRegion` writes to a destination that is not in `COPY_DEST`, and
- the barrier declares a `StateBefore` that does not match the actual state.

Both are undefined behaviour.

**Fix:** track the state on `Texture` and transition back before the copy.

```rust
struct Texture {
    // ...
    state: D3D12_RESOURCE_STATES,
}

// in upload_texture, before CopyTextureRegion:
let pre_barriers = if tex_state == D3D12_RESOURCE_STATE_COPY_DEST {
    Vec::new()
} else {
    vec![util::create_barrier(&tex_resource, tex_state, D3D12_RESOURCE_STATE_COPY_DEST)]
};
if !pre_barriers.is_empty() {
    self.command_list.ResourceBarrier(&pre_barriers);
}
```

---

## 3. Textures are shared across two queues without a `COMMON` ownership transfer

`TextureHeap::new` creates **its own** command queue:

```rust
let command_queue = unsafe {
    device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
        Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
        ...
    })
}?;
```

so a texture is written on the TextureHeap's queue and sampled on the render
engine's queue. D3D12 requires a resource shared between queues to pass through
`D3D12_RESOURCE_STATE_COMMON` to transfer ownership — that is the transition
which resolves any driver-side compression metadata. Going straight from
`COPY_DEST` to `PIXEL_SHADER_RESOURCE` on the upload queue skips it.

**Fix:** create the resource in `COMMON`, leave it in `COMMON` after upload, and
let implicit state promotion handle both the copy and the sampling on whichever
queue needs it.

---

## Not a hudhook bug, but worth recording

While chasing this I also hit a texture corruption that turned out to be
**triggered by the texture being fully opaque (alpha == 255)** in one specific
game (Black Myth: Wukong 1.0.20+, which integrated FSR4 frame generation).
Identical pixels uploaded with alpha 200 sample perfectly; a texture that
normally renders fine breaks as soon as its alpha is forced to 255.

I verified it is **not** an upload problem: the corruption survives
`CopyTextureRegion`, `GetCopyableFootprints`-derived layouts, and a
CPU-writable `D3D12_HEAP_TYPE_CUSTOM` heap with `WriteToSubresource` (which
bypasses the upload buffer, the command queue, the fence and the barriers
entirely). Reading the upload buffer back immediately before the copy shows
byte-perfect data.

Most likely the game's frame-generation UI handling treating fully-opaque
overlay pixels as something to reproject. Recording it here in case another
hudhook user hits the same thing — the workaround is to clamp alpha to 254.
