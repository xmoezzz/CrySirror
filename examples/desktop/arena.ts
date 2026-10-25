// SPDX-License-Identifier: MPL-2.0
// Offline recipe: the player consumes the resulting schema-2 JSON, not this file.
type Vec3 = [number, number, number];
type Rgba = [number, number, number, number];

function buildProject() {
    const meshes = [{
        name: "floor", geometry: { kind: "cube" },
        translation: [0, -1, 0] as Vec3, scale: [12, 0.2, 12] as Vec3,
        color: [65, 75, 90, 255] as Rgba,
    }];
    for (let row = 0; row < 3; row++) {
        for (let column = 0; column < 3; column++) {
            meshes.push({
                name: `pillar_${row}_${column}`, geometry: { kind: "cube" },
                translation: [(column - 1) * 4, 0, (row - 1) * 4],
                scale: [0.6, 2, 0.6], color: [110, 145, 175, 255],
            });
        }
    }
    return {
        schema: 2, name: "CrySirror arena blockout", content_revision: "arena-1",
        canvas: [1280, 720], assets: "assets",
        clients: [{ name: "arena", adapter: "crysirror-script-v1", game: "arena", program: [{ op: "halt" }] }],
        scene3d: {
            camera: { eye: [11, 9, 14], target: [0, 0, 0], up: [0, 1, 0], fov_y_degrees: 55, near: 0.1, far: 100 },
            meshes,
        },
    };
}
