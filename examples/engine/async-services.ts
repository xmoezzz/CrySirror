// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
// Run with an explicitly supplied runtime font; no native VM or plugin required.
async function onInit(): Promise<void> {
  ui.setMenu({id: 1, rect: [0, 0, 960, 24], font: "default", items: [
    {id: 2, label: "Text", children: [
      {id: 3, label: "Pause", radio: true, group: 1},
      {id: 4, label: "Continue", radio: true, group: 1, checked: true},
      {id: 5, label: "Finish"}
    ]},
    {id: 6, label: "Save"}
  ]});
  const surface = await text.drawAsync("intro",
    "[b]CrySirror[/b]: shared menus, worker text and Promise/await.[event=shown]", {
      style: {fonts: ["default"], size: 28, color: [255, 255, 255, 255], bold: false},
      layout: {canvas: [900, 150]}, graphemes_per_second: 18,
      raster: {outline: {width: 1, color: [0, 0, 0, 255]}}
    });
  layers.put({id: 100, rect: [30, 100, 900, 150],
    paint: {kind: "image", asset: surface}});
  engine.log("async-demo-ready");
}

async function onUpdate(): Promise<void> {
  for (const event of ui.drainEvents()) {
    if (event.kind !== "menu_activate") continue;
    switch (event.widget.id) {
      case 3: text.pause("intro", true); break;
      case 4: text.pause("intro", false); break;
      case 5: text.finish("intro"); break;
      case 6: engine.log(`saved ${await save.saveAsync("async-demo.save")} bytes`); break;
    }
  }
  for (const event of text.drainEvents("intro")) {
    if (event.kind === "marker") engine.log(`text marker: ${event.name}`);
  }
}
