# SPDX-License-Identifier: MPL-2.0
"""Self-authored layout fixtures. No game data or proprietary decoder dependency."""
import struct
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
from gow3_scene import WadError, dds_full, dds_preview, edge_indices, mesh


def triangle_mesh():
    b = bytearray(0x400)
    def put(fmt, at, *values): struct.pack_into(fmt, b, at, *values)
    put('>II', 0, 0x1000d, len(b))
    put('>I', 24, 48)  # top = 64
    put('>H', 64, 1)
    put('>i', 76, 8)  # group pointer table = 84
    put('>i', 84, 20)  # group = 104
    put('>II', 104, 23, 1)
    put('>HH', 112, 2, 1)  # material slot is independent of surface ordinal
    put('>I', 132, 152)  # geometry = 256
    put('>HH', 256, 0, 1)
    put('>i', 284, 228)  # config = 512
    put('>HH', 520, 3, 3)
    put('>i', 532, 300)  # index stream = 832
    put('>H', 536, 16)
    put('>i', 544, 224)  # vertex stream = 768
    put('>H', 552, 48)
    put('>i', 580, -200)  # attribute descriptor = 380
    b[380:384] = bytes([2,16,2,0])
    b[388:404] = bytes([0,2,3,1,12,0,0,0,12,6,1,2,4,2,0,0])
    for i, p in enumerate([(0,0,0),(1,0,0),(0,0,1)]):
        put('>3fI', 768+i*16, *p, 1023 << 11)
    b[832:848] = bytes([0,0,0,0,0,1,0,0,0,0xc0])+bytes(6)
    return b


class GoWSceneTests(unittest.TestCase):
    def test_topology_controls_reuse_and_winding(self):
        self.assertEqual(edge_indices(bytes([0,0,0,0,0,1,0,0,0,0xc6]),12,6),
                         [0,2,1,0,3,2,3,4,2,2,5,3])
        self.assertEqual(edge_indices(bytes([0,3,0,0,0,1,2,0,0x1c,0xf0,0x24]),6,3),[0,2,1,0,1,2])

    def test_eight_lane_negative_delta_predictor(self):
        values=[2,1,0,2,1,0,2,1,0]
        deltas=[v-(values[i-8] if i>=8 else 0)+2 for i,v in enumerate(values)]
        bits=''.join(f'{v:03b}' for v in deltas).ljust(32,'0')
        data=struct.pack('>HHHBB',9,2,2,3,0)+bytes([0x1f,0xf0,0xff])+int(bits,2).to_bytes(4,'big')
        expected=[0,2,1]
        for i in range(0,len(values),3):expected.extend([values[i],values[i+2],values[i+1]])
        self.assertEqual(edge_indices(data,12,3),expected)
        for invalid in [data[:-1],data[:7]+b'\1'+data[8:],data[:8]+b'\xff'+data[9:]]:
            with self.assertRaises(WadError):edge_indices(invalid,12,3)

    def test_declared_geometry_offsets_and_packed_normals(self):
        b=triangle_mesh();parts=mesh(b,'self-authored')
        self.assertEqual(parts[0]['indices'],[0,2,1])
        self.assertEqual(parts[0]['positions'],[[0,0,0],[1,0,0],[0,0,1]])
        self.assertEqual(parts[0]['normals'],[[0,1,0]]*3)
        self.assertEqual(parts[0]['material_slot'],2)
        self.assertFalse(parts[0]['uv_supported'])
        struct.pack_into('>i',b,532,10000)
        with self.assertRaises(WadError):mesh(b,'invalid pointer')

    def test_unknown_vertex_layout_fails(self):
        b=triangle_mesh();b[389]=99
        with self.assertRaises(WadError):mesh(b,'unknown attribute')

    def test_inline_dds_mip_selection_and_vendor_flags(self):
        b=bytearray(204);struct.pack_into('>I',b,0,0x10007);struct.pack_into('>I',b,64,3)
        b[76:80]=b'DDS '
        for at,v in [(4,124),(8,0x900a1007),(12,8),(16,8),(28,4),(76,32),(80,4)]:struct.pack_into('<I',b,76+at,v)
        b[160:164]=b'DXT1';b.extend(bytes(range(24)))  # inline 4x4, 2x2, 1x1
        result=dds_preview(b,2)
        self.assertEqual(struct.unpack_from('<II',result,12),(2,2))
        self.assertEqual(struct.unpack_from('<I',result,28)[0],2)
        self.assertEqual(result[128:],bytes(range(8,24)))
        self.assertEqual(struct.unpack_from('<I',result,8)[0],0xa1007)
        with self.assertRaises(WadError):dds_preview(b[:-1])
        struct.pack_into('<I',b,76+112,0x200)
        with self.assertRaisesRegex(WadError,'cubemap'):dds_preview(b)

    def test_full_mip_chain_joins_ts_stream_and_inline_mips(self):
        # 16x16 DXT1, 5 mips: 16 and 8 streamed (128+32 bytes), 4/2/1 inline (8 each).
        b=bytearray(204);struct.pack_into('>I',b,0,0x10007);struct.pack_into('>I',b,60,100);struct.pack_into('>I',b,64,3)
        b[76:80]=b'DDS '
        for at,v in [(4,124),(8,0x1007),(12,16),(16,16),(28,5),(76,32),(80,4)]:struct.pack_into('<I',b,76+at,v)
        b[160:164]=b'DXT1';b.extend(bytes([7])*24)
        ts=bytes(100)+bytes([1])*128+bytes([2])*32+bytes(9)
        full=dds_full(bytes(b),ts,16)
        self.assertEqual(struct.unpack_from('<II',full,12),(16,16));self.assertEqual(struct.unpack_from('<I',full,28)[0],5)
        self.assertEqual(full[128:],bytes([1])*128+bytes([2])*32+bytes([7])*24)
        half=dds_full(bytes(b),ts,8)
        self.assertEqual(struct.unpack_from('<II',half,12),(8,8));self.assertEqual(half[128:],bytes([2])*32+bytes([7])*24)
        # Without a TS stream the inline mips remain the source.
        self.assertEqual(struct.unpack_from('<II',dds_full(bytes(b),None,16),12),(4,4))
        with self.assertRaisesRegex(WadError,'TS stream range'):dds_full(bytes(b),ts[:200],16)


if __name__ == '__main__':unittest.main()
