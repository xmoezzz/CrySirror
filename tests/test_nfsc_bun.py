# SPDX-License-Identifier: MPL-2.0
"""tools/nfsc_bun.py on synthetic data only (no game bytes in the repo)."""
from pathlib import Path
import struct, sys, unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import nfsc_bun as b


def chunk(cid, body):
    return struct.pack("<II", cid, len(body)) + body


def section(name, number, offset, size):
    raw = name.encode().ljust(8, b"\0") + struct.pack("<HBB", number, 0, 0)
    raw += struct.pack("<IIIIIIi", 0, 1, offset, size, size, size, 10)
    raw += struct.pack("<fffI", 1.5, -2.0, 30.0, 7)
    return raw.ljust(b.SECTION_SIZE, b"\0")


class BunTests(unittest.TestCase):
    def test_nested_chunks_and_histogram(self):
        data = chunk(0x80134000, chunk(0x80134010, chunk(0x00134B01, b"\1" * 8)) + chunk(0, b"")) + chunk(0x00034110, b"")
        walked = [(d, c) for d, _, c, _ in b.chunks(data)]
        self.assertEqual(walked, [(0, 0x80134000), (1, 0x80134010), (2, 0x00134B01), (1, 0), (0, 0x00034110)])
        st = b.stats(data)
        self.assertEqual(st["0x00134b01"]["bytes"], 8)
        self.assertEqual(st["0x00034110"]["name"], "track_streaming_sections")

    def test_overrun_fails_visibly(self):
        with self.assertRaises(b.BunError):
            list(b.chunks(struct.pack("<II", 0x80134000, 100) + b"\0" * 8))

    def test_sections_layout(self):
        data = chunk(0x00034110, section("A41", 141, 4096, 1000) + section("T51", 2051, 8192, 2000))
        s = b.sections(data)
        self.assertEqual([(x["name"], x["number"], x["offset"], x["size"]) for x in s], [("A41", 141, 4096, 1000), ("T51", 2051, 8192, 2000)])
        self.assertEqual(s[0]["center"], [1.5, -2.0])
        with self.assertRaises(b.BunError):
            b.sections(chunk(0x00034110, b"\0" * 10))


if __name__ == "__main__":
    unittest.main()


import nfsc_geometry as g
import nfsc_textures as tx


def solid_section():
    """Header, mesh entry and vertices of one triangle (shader type 0)."""
    verts = b"".join(struct.pack("<6f", x, y, z, 0, 0, 1) + bytes([10, 20, 30, 255]) + struct.pack("<2f", u, v)
                     for x, y, z, u, v in [(0, 0, 0, 0, 0), (1, 0, 0, 1, 0), (0, 1, 0, 0, 1)])
    entry = bytearray(144)
    struct.pack_into("<I", entry, 64, 3)
    struct.pack_into("<2I", entry, 96, 1, 0)
    struct.pack_into("<I", entry, 124, 3)
    header = bytearray(160) + b"TEST_SOLID\0".ljust(32, b"\0")
    return header, entry, verts


class GeometryTests(unittest.TestCase):
    def build(self):
        header, entry, verts = solid_section()
        # Assemble with correct absolute alignment by building front to back.
        data = bytearray()
        def put(cid, body, align):
            nonlocal data
            if align:
                pad = (-(len(data) + 8)) % 16
                body = b"\x11" * pad + body
            data += struct.pack("<II", cid, len(body)) + body
        # container headers are patched afterwards
        data += struct.pack("<II", 0x80134010, 0)
        put(0x00134011, bytes(header), True)
        put(0x00134012, struct.pack("<II", 0x1234, 0), False)
        mesh_at = len(data)
        data += struct.pack("<II", 0x80134100, 0)
        put(0x00134900, bytes(32), True)
        put(0x00134B02, bytes(entry), True)
        put(0x00134B03, struct.pack("<3H", 0, 1, 2), True)
        put(0x00134B01, verts, True)
        put(0x00134C02, b"MAT_A\0\0\0", False)
        struct.pack_into("<I", data, mesh_at + 4, len(data) - mesh_at - 8)
        struct.pack_into("<I", data, 4, len(data) - 8)
        return bytes(data)

    def test_solid_decodes_to_y_up_mesh_with_material_and_texture(self):
        stats = {}
        (s,) = list(g.solids(self.build(), stats))
        self.assertEqual(s["name"], "TEST_SOLID")
        self.assertEqual(s["textures"], [0x1234])
        (m,) = s["meshes"]
        self.assertEqual(m["material"], "MAT_A")
        self.assertEqual(m["indices"], [0, 1, 2])
        self.assertEqual(m["positions"][2], (0.0, 0.0, -1.0), "Z-up (0,1,0) becomes Y-up (0,0,-1)")
        self.assertEqual(m["normals"][0], (0.0, 1.0, -0.0))
        self.assertEqual(m["colors"][0][:3], (30 / 255, 20 / 255, 10 / 255), "BGRA order")
        self.assertEqual(stats["unit_normals"], stats["vertices"])
        self.assertEqual(g.diffuse_key(s, m), 0x1234)

    def test_unknown_shader_type_fails_visibly(self):
        data = bytearray(self.build())
        i = data.find(struct.pack("<I", 0x00134B02))
        entry_at = i + 8 + ((-(i + 8)) % 16)
        struct.pack_into("<I", data, entry_at + 48, 99)
        with self.assertRaises(g.GeometryError):
            list(g.solids(bytes(data)))


class TextureTests(unittest.TestCase):
    def tpk(self, comp=34, w=8, h=8, mips=2):
        img = bytes(range(256)) * 2
        size = sum(max(1, (max(1, w >> i) + 3) // 4) * max(1, (max(1, h >> i) + 3) // 4) * (8 if comp == 34 else 16) for i in range(mips))
        rec = bytearray(88)
        struct.pack_into("<I", rec, 12, 0xBEEF)
        struct.pack_into("<i", rec, 20, 0)
        struct.pack_into("<i", rec, 28, size)
        struct.pack_into("<HH", rec, 40, w, h)
        rec[46], rec[50], rec[57], rec[58] = comp, mips, 1, 0
        rec += bytes([5]) + b"NAME\0" + b"\0" * 2
        info = chunk(0x33310001, b"\0" * 4 + b"PACK".ljust(28, b"\0")) + chunk(0x33310004, bytes(rec))
        data = bytearray(chunk(0xB3310000, info))
        vram_head = 8 + len(data) + 8 + 8  # outer B3300000, B3320000 and 33320002 headers
        pad = (-vram_head) % 128
        vram = chunk(0x33320002, b"\x11" * pad + img[:size])
        data += chunk(0xB3320000, vram)
        return chunk(0xB3300000, bytes(data)), img[:size]

    def test_pack_records_and_dds(self):
        raw, img = self.tpk()
        (name, (t,)), = list(tx.packs(raw))
        self.assertEqual((name, t["name"], t["key"], t["width"], t["mips"], t["alpha"], t["blend"]), ("PACK", "NAME", 0xBEEF, 8, 2, "punchthru", "src_copy"))
        self.assertEqual(t["image"], img)
        d = tx.dds(t)
        self.assertEqual(d[:4], b"DDS ")
        self.assertEqual(struct.unpack_from("<I", d, 28)[0], 2)
        self.assertEqual(d[84:88], b"DXT1")
        self.assertEqual(d[128:], img)
        small = tx.dds(t, max_size=4)
        self.assertEqual(struct.unpack_from("<II", small, 12), (4, 4))
