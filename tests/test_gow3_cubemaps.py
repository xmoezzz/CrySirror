# SPDX-License-Identifier: MPL-2.0
"""Synthetic checks for tools/gow3_cubemaps.py (no game data)."""
import struct, sys, unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
import numpy as np
import gow3_cubemaps as c


def synthetic_faces(n=32):
    """A smooth function of direction rendered with the D3D face convention."""
    t = np.linspace(-1 + 1 / n, 1 - 1 / n, n)
    sc, tc = np.meshgrid(t, t)
    faces = []
    for f in range(6):
        x, y, z = c.face_dir(f, sc, tc)
        norm = np.sqrt(x * x + y * y + z * z)
        rgb = np.stack([(x / norm + 1) * 127, (y / norm + 1) * 127, (z / norm + 1) * 127, np.full_like(x, 255)], -1)
        faces.append(rgb.astype(np.uint8))
    return faces


class Cubemaps(unittest.TestCase):
    def test_seams_confirm_the_convention_and_reject_permutations(self):
        faces = synthetic_faces()
        good = c.seam_error(faces, list(range(6)))
        swapped = c.seam_error(faces, [1, 0, 2, 3, 4, 5])
        self.assertLess(good, 6)
        self.assertGreater(swapped, good * 3)

    def test_equirect_matches_the_engine_direction_convention(self):
        img = c.equirect(synthetic_faces(), 64)
        # u=0.25 is -Z in the prefilter convention (atan2(z, x) = -pi/2); v=0.5 is the horizon.
        px = img[16, 16]
        self.assertLess(px[2], px[0])  # z channel low → -Z
        self.assertGreater(img[1, 32][1], img[30, 32][1])  # top row is +Y

    def test_split_faces_checks_layout(self):
        n, mips = 8, 4
        chain = sum(max(1, (max(1, n >> i) + 3) // 4) ** 2 * 16 for i in range(mips))
        header = bytearray(128)
        header[0:4] = b'DDS '
        for at, v in [(4, 124), (8, 0x900A1007), (12, n), (16, n), (28, mips), (76, 32), (112, 0xFE00)]:
            struct.pack_into('<I', header, at, v)
        header[84:88] = b'DXT5'
        wrapper = bytearray(76)
        struct.pack_into('>I', wrapper, 0, 0x10007)
        struct.pack_into('>I', wrapper, 64, 0xFFFFFFFF)
        payload = bytes(wrapper) + bytes(header) + bytes(6 * chain)
        faces, meta = c.split_faces(payload)
        self.assertEqual((len(faces), meta['face_bytes']), (6, chain))
        self.assertEqual(struct.unpack_from('<I', faces[0], 112)[0], 0)
        with self.assertRaises(c.WadError):
            c.split_faces(payload[:-1])
        bad = bytearray(payload)
        struct.pack_into('<I', bad, 76 + 112, 0)
        with self.assertRaises(c.WadError):
            c.split_faces(bytes(bad))


if __name__ == '__main__':
    unittest.main()
