# SPDX-License-Identifier: MPL-2.0
"""tools/road_centerline.py on a synthetic L-shaped road."""
from pathlib import Path
import json, struct, sys, tempfile, unittest
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import road_centerline as rc


def write_road(dir: Path) -> Path:
    # Two 10 m wide strips: x 0..100 at z -5..5, then z 0..80 at x 90..100 (L shape), y = 2.
    quads = [((0, -5), (100, -5), (100, 5), (0, 5)), ((90, 5), (100, 5), (100, 80), (90, 80))]
    verts, idx = [], []
    for q in quads:
        b = len(verts)
        verts += [(x, 2.0, z) for x, z in q]
        idx += [b, b + 1, b + 2, b, b + 2, b + 3]
    blob = struct.pack(f"<{len(verts) * 3}f", *[c for v in verts for c in v]) + struct.pack(f"<{len(idx)}I", *idx)
    (dir / "s.bin").write_bytes(blob)
    g = {"asset": {"version": "2.0"}, "nodes": [{"name": "RD_TEST", "mesh": 0}, {"name": "OTHER", "mesh": 0}],
         "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
         "accessors": [{"bufferView": 0, "componentType": 5126, "count": len(verts), "type": "VEC3"},
                       {"bufferView": 1, "componentType": 5125, "count": len(idx), "type": "SCALAR"}],
         "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": len(verts) * 12},
                         {"buffer": 0, "byteOffset": len(verts) * 12, "byteLength": len(idx) * 4}],
         "buffers": [{"uri": "s.bin", "byteLength": len(blob)}]}
    p = dir / "s.gltf"
    p.write_text(json.dumps(g))
    return p


class CentrelineTests(unittest.TestCase):
    def test_route_is_centred_and_spans_the_road(self):
        with tempfile.TemporaryDirectory() as tmp:
            tris = rc.road_triangles(write_road(Path(tmp)), "RD_")
            self.assertEqual(len(tris), 4, "only nodes with the prefix")
            height, lo = rc.rasterize(tris, 1.0)
            pts, rep = rc.centreline(height, lo, 1.0, (1, 2, 0), spacing=2.0)
            self.assertEqual(rep["reachable"], rep["cells"])
            a = np.array(pts)
            # Along the first strip the route stays near z = 0; along the second near x = 95.
            first = a[(a[:, 0] > 15) & (a[:, 0] < 80)]
            second = a[(a[:, 2] > 20) & (a[:, 2] < 70)]
            self.assertLess(np.abs(first[:, 2]).max(), 1.5)
            self.assertLess(np.abs(second[:, 0] - 95).max(), 1.5)
            self.assertGreater(a[-1, 2], 70, "ends at the far end of the second strip")
            self.assertTrue(np.allclose(a[:, 1], 2.0))
            self.assertTrue(80 < rep["length_m"] < 190)


if __name__ == "__main__":
    unittest.main()
