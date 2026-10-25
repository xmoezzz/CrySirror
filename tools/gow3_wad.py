#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Inspect the observed GoW III PS3 big-endian WAD framing, without guessing mesh layouts.

The profile has a 64-byte record header (kind/size/name), 16-byte alignment,
version record 0x00140000 and explicit group markers. Record kinds are opaque.
EntityCount is quantity metadata, not a payload length. This module indexes
records only; geometry is handled by the separate scene frontend.
"""
from pathlib import Path
import argparse,json,struct
from collections import Counter

class WadError(ValueError):pass

def inspect(data:bytes)->dict:
    if not 64<=len(data)<=256*1024*1024:raise WadError('WAD size outside inspection profile')
    if data[:4]!=bytes.fromhex('00140000'):raise WadError('unknown WAD version/layout')
    records=[];offset=0;depth=0
    while offset<len(data):
        if offset+64>len(data):raise WadError('truncated record header')
        kind,size=struct.unpack_from('>II',data,offset)
        name=data[offset+8:offset+64].split(b'\0')[0].decode('ascii',errors='strict')
        metadata=None
        if kind==0:
            if name!='EntityCount':raise WadError(f'unsupported metadata {name!r} at 0x{offset:x}')
            metadata=size;size=0
            if metadata>1_000_000:raise WadError(f'EntityCount limit at 0x{offset:x}')
        end=offset+64+size;aligned=(end+15)&~15
        if aligned>len(data):raise WadError(f'record payload/alignment outside file at 0x{offset:x} kind 0x{kind:x}')
        if kind==0x20000:
            depth+=1
        elif kind==0x30000:
            if depth==0:raise WadError('unbalanced group end')
            depth-=1
        records.append({'offset':offset,'kind':f'0x{kind:08x}','size':size,'name':name,**({'metadata_value':metadata}if metadata is not None else {})})
        if len(records)>1_000_000 or depth>1024:raise WadError('record/depth limit')
        offset=aligned
    if depth:raise WadError('unclosed WAD groups')
    return {'bytes':len(data),'consumed':offset,'record_count':len(records),'kinds':dict(Counter(r['kind']for r in records)),'records':records,'mesh_decoder':False,'scene_renderer':False}

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('wad',type=Path);a=p.parse_args();print(json.dumps(inspect(a.wad.read_bytes()),indent=2))
if __name__=='__main__':main()
