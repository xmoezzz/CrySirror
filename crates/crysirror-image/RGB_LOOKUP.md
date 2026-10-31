# Shared RGB lookup effect

`ImageEffect::RgbLookup { spec: RgbLookup }` is accepted by the existing shared
`ImageMetadata::begin_process` Asset/file and immutable-resource inputs, and by
public `Bitmap.beginEffect`. Its JSON kind is `rgb_lookup`. The spec has `red`,
`green` and `blue` arrays containing exactly 256 byte values each, plus explicit
`profile: "straight_rgba"`.

Each stored encoded RGB byte independently addresses its channel table. Every
alpha byte is copied exactly, including zero alpha; hidden RGB also undergoes
the lookup. This is neither gamma conversion nor a colour-space inference.
ImageSurface and ImageResource have straight-alpha semantics and no association
metadata. `premultiplied_rgba` is recognized solely for an explicit validation
error; it never admits an ImageJob or emits premultiplied output disguised as a
normal surface. Unknown profiles also fail. There is no alpha heuristic.

The source remains immutable. Limits are 768 LUT bytes and the shared effect
limit of 4,194,304 pixels (at most 16 MiB per RGBA image). Scratch is one output
clone with row cancellation fences; the shared ImageJob reserves its existing
worker/memory budgets before admission. Malformed tables, incompatible profiles
and cancellation publish no result. An Asset input keeps its checked size and
optional fingerprint, so a changed encoded source fails before a Ready receipt.

The engine owns decoding and pixel execution. Owner publication of a Ready
image uses HostServices.publish_image_resource: client/layout/budget validation,
a unique immutable path and retained RGBA Arc/resource lease, without PNG,
hashing, scans or pixel copies on the owner. Foreign image resources and foreign
publication/release paths cannot authorize access to another client's pixels.
Releasing a path requires callers to first remove active logical references;
frozen frames retain their resource leases independently.

The kernel is original CrySirror code and has no RealLive dependency. Native
TCC headers, filename suffix numbering and Gameexe lookup stay in the thin
RealLive metadata policy; no native VM contains a second lookup/compositor.
