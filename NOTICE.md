# Notices

Most CrySirror implementation is newly authored code. The Siglus resource port
in crates/crysirror-siglus derives from xmoezzz/siglus_rs at
5f358ef1360b0439e92aed2b749e8f25d8721954 under MPL-2.0; see that crate's UPSTREAM.md
and per-file notices. The monorepo root LICENSE contains the full MPL-2.0 text.
Shared CPU input/animation/projection and UI coordinate algorithms are also
migrated into crates/crysirror-scene from Siglus at the same commit and RFVP
at 062fec82033b554b9eb3b5da069518c0c96a354e (MPL-2.0). See that crate's
UPSTREAM.md, per-file notices and upstream-files.json for exact sources/changes.

The tiny
checkerboard/marker PNGs and tone.wav are self-authored test assets covered by
the repository's MPL-2.0 license. They are not extracted game assets.

No user game resource, game-specific key, credential,
proprietary SDK or font binary is included in this snapshot. The Siglus type-3
JPEG format's public fixed scrambling table is retained as algorithm code.

Intended upstream integration targets:

- https://github.com/xmoezzz/rfvp
- https://github.com/xmoezzz/siglus_rs

When importing their actual implementation, preserve per-file attribution,
upstream license text, exact origin commits and applicable dependency notices.
This workspace's license does not relicense upstreams, third-party crates,
fonts, game content or SDKs.

Cargo dependencies are external. Dependency resolution and local builds now
succeed; their complete resolved licenses/notices have not been audited here.
The packager includes CrySirror's own notice/license and current engine source,
not a certification of complete binary-distribution license compliance. Verify
source/binary correspondence and third-party notices before a public release.

The optional `crysirror-formats-arc` plugins translate selected archive decoders
from vn-tools/arc_unpacker at commit
456834ecf2e5686813802c37efd829310485c57d, Copyright (C) 2016 rr-.
They retain GPL-3.0-or-later attribution and the upstream license text in that
crate. The `arc-formats` host feature is explicit; the complete registration
inventory is a migration ledger, not a declaration that every decoder is ported.
See crates/crysirror-formats-arc/UPSTREAM.md and docs/RESOURCES.md.
The shared physics backend depends on Rapier3D 0.22.0; source, checksum and
reproducible behavior/performance evidence are recorded in ADR 0045.

The upstream fetch tool creates reference checkouts on the developer's machine;
use --source-root outside the deliverable tree. Those checkouts are not included
in this source export. No fonts may be added to an
assistant-generated delivery merely because they happen to be installed locally.

The optional KiriKiri adapter vendors `krkr-tjs2` and `krkr-kag` from
liulifox233/Kirakira at commit c451918023477620ff9055a2eb8a6714b947d9c1
(crates/krkr-upstream, unmodified sources, SHA-256 in SOURCE.json), and
crates/crysirror-krkr/src/constants.rs is ported from that commit's
`krkr-engine/src/globals/constants.rs`. They are licensed AGPL-3.0-or-later
(crates/krkr-upstream/LICENSE). `crysirror-krkr` is AGPL-3.0-or-later too.
None of them is in a default build; a binary built with the desktop `krkr`
feature is covered by the AGPL, including its section 13 network clause.
The krkr-demo images and scripts are self-authored.
