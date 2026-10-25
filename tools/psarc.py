#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Read-only, bounded PSARC 1.4/zlib inspection and selected-file extraction.

Uses the archive's block-size table; never searches for plausible geometry.
Output is explicit and source archives are never modified. Game content must
remain outside the CrySirror source export. See docs/LOCAL_GAME_IMPORT.md.
"""
from __future__ import annotations
import argparse
from dataclasses import dataclass
import json
from pathlib import Path
import struct
import zlib
import os,tempfile

class ArchiveError(ValueError):
    pass

def check(condition, message):
    if not condition:
        raise ArchiveError(message)

@dataclass(frozen=True)
class Entry:
    digest: bytes
    block: int
    length: int
    offset: int

class Archive:
    def __init__(self, path: Path):
        self.path = Path(path)
        self.size = self.path.stat().st_size
        with self.path.open('rb') as f:
            h = f.read(32)
            check(len(h) == 32, 'truncated PSARC header')
            magic, self.version, method, toc, stride, count, self.block_size, self.flags = struct.unpack('>4sI4s5I', h)
            check(magic == b'PSAR', 'not a PSARC archive')
            check(self.version == 0x10004 and method == b'zlib', 'unsupported PSARC version/compression')
            check(self.flags & ~3 == 0, 'unsupported PSARC flags')
            check(stride == 30 and 0 < count <= 1_000_000, 'unsupported TOC entry size/count')
            check(256 <= self.block_size <= 16*1024*1024 and self.block_size & (self.block_size-1) == 0, 'invalid block size')
            base = 32 + stride*count
            check(base <= toc <= min(self.size,64*1024*1024), 'invalid TOC size')
            raw = f.read(toc-32)
            check(len(raw) == toc-32, 'truncated TOC')
        self.entries = []
        for i in range(count):
            b = raw[i*stride:(i+1)*stride]
            e = Entry(b[:16],int.from_bytes(b[16:20],'big'),int.from_bytes(b[20:25],'big'),int.from_bytes(b[25:30],'big'))
            check(toc <= e.offset <= self.size, 'entry offset outside payload')
            self.entries.append(e)
        width = max(1, ((self.block_size-1).bit_length()+7)//8)
        blocks = raw[stride*count:]
        check(len(blocks)%width == 0, 'partial compression table entry')
        self.blocks = [int.from_bytes(blocks[i:i+width],'big') for i in range(0,len(blocks),width)]
        for e in self.entries:
            check(e.block+(e.length+self.block_size-1)//self.block_size <= len(self.blocks), 'entry block range outside table')
        check(self.entries[0].length <= 16*1024*1024,'name list too large')
        names = self.read(0,16*1024*1024).decode('utf-8',errors='strict').splitlines()
        check(len(names) == count-1 and len(set(names)) == len(names),'file-name count/uniqueness mismatch')
        self.names = names
        self.by_name = {name:i+1 for i,name in enumerate(names)}

    def iter_entry(self, index: int):
        check(type(index) is int and 0<=index<len(self.entries),'entry index outside archive')
        e = self.entries[index]
        remaining = e.length
        with self.path.open('rb') as f:
            f.seek(e.offset)
            for block in range(e.block,e.block+(e.length+self.block_size-1)//self.block_size):
                expected = min(self.block_size,remaining)
                stored = self.blocks[block]
                n = stored or self.block_size
                check(f.tell()+n <= self.size,'block exceeds archive')
                b = f.read(n)
                check(len(b) == n,'truncated block')
                if stored and stored != expected:
                    d = zlib.decompressobj()
                    data = d.decompress(b,expected+1)
                    check(d.eof and not d.unused_data and not d.unconsumed_tail,'invalid zlib block')
                else:
                    data = b
                check(len(data) == expected,'decoded block length mismatch')
                remaining -= expected
                yield data
            check(remaining == 0,'entry size mismatch')

    def read(self,index:int,limit=64*1024*1024)->bytes:
        check(self.entries[index].length <= limit,'entry exceeds read limit')
        return b''.join(self.iter_entry(index))

    def prefix(self,index:int,n=256)->bytes:
        out = bytearray()
        for b in self.iter_entry(index):
            out.extend(b[:max(0,n-len(out))])
            if len(out) >= n:
                break
        return bytes(out)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command',choices=['list','extract'])
    parser.add_argument('archive',type=Path)
    parser.add_argument('--entry')
    parser.add_argument('--output',type=Path)
    args = parser.parse_args()
    arc = Archive(args.archive)
    if args.command == 'list':
        print(json.dumps({'archive':args.archive.name,'version':arc.version,'flags':arc.flags,'block_size':arc.block_size,'files':[{'name':n,'bytes':arc.entries[i+1].length} for i,n in enumerate(arc.names)]},ensure_ascii=False,indent=2))
    else:
        check(args.entry in arc.by_name and args.output is not None,'extract requires an existing --entry and explicit --output')
        out = args.output.resolve()
        source = Path(__file__).resolve().parents[1]
        check(not out.is_relative_to(source),'game resources must be extracted outside the source export')
        check(out != args.archive.resolve(),'cannot overwrite source archive')
        out.parent.mkdir(parents=True,exist_ok=True)
        # Stage the complete decode; publish exclusively without replacing an
        # existing file or leaving a successful-looking partial extraction.
        temporary=None
        try:
            with tempfile.NamedTemporaryFile(dir=out.parent,prefix='.psarc-',delete=False) as stream:
                temporary=Path(stream.name)
                for b in arc.iter_entry(arc.by_name[args.entry]):stream.write(b)
                stream.flush();os.fsync(stream.fileno())
            os.link(temporary,out)
        finally:
            if temporary is not None:temporary.unlink(missing_ok=True)
        print(json.dumps({'output':str(out),'bytes':out.stat().st_size}))

if __name__ == '__main__':
    main()
