// SPDX-License-Identifier: MPL-2.0
// CrySirror launcher: the game library shared by every host (desktop,
// web, PS Vita, Switch). An authored CrySirror project run by the shared
// runtime and renderer; crysirror-launch `library` generates games.json and
// the card, glow and backdrop images. The chosen entry id is written to the
// save file `launch.json`, then the launcher quits and the host opens it.

const ENTER = 1, LEFT = 1 << 2, RIGHT = 1 << 3, UP = 1 << 4, DOWN = 1 << 5;
const PAGE_UP = 1 << 23, PAGE_DOWN = 1 << 24, MOUSE = 1 << 16;

let library = null;
let W = 960, H = 544;
let selected = 0;
let scroll = 0;          // animated first-card offset, in cards
let focus = 0;           // animated selection, for the glow
let launching = -1;      // launch animation start time, or -1
let lastTime = 0;
let lastTap = null;
let cardW = 0, cardH = 0, gap = 0, rowY = 0, rowX = 0;
const LINE = "simple-horizontal-unwrapped-v1";

const L = {
  backdrop: 1, glow: 2, header: 3, count: 4, title: 5, subtitle: 6,
  hintPill: 7, hintA: 8, hintAText: 9, hintMove: 10, empty: 11, emptyHelp: 12,
  fade: 13, cards: 1000, initials: 2000, names: 3000,
};

function px(v) { return Math.round(v); }

function onInit() {
  library = JSON.parse(fs.readText("games.json"));
  W = library.canvas[0];
  H = library.canvas[1];
  cardW = library.card[0];
  cardH = library.card[1];
  gap = px(W * 0.025);
  rowX = px(W * 0.07);
  rowY = px(H * 0.235);
  const s = library.scale;

  layers.put({id: L.backdrop, rect: [0, 0, W, H], order: [0, 0],
    paint: {kind: "image", asset: "backdrop.png"}});
  layers.put({id: L.header, rect: [rowX, px(36 * s), W, px(40 * s)], order: [10, 0],
    paint: {kind: "text", text: "CrySirror", font: "default", size: px(26 * s), color: [255, 255, 255, 255]}});
  const count = library.games.length;
  layers.put({id: L.count, rect: [px(W - rowX - 90 * s), px(42 * s), px(120 * s), px(30 * s)], order: [10, 1],
    paint: {kind: "text", text: count === 1 ? "1 game" : `${count} games`, font: "default", size: px(16 * s), color: [190, 196, 230, 200]}});

  if (count === 0) {
    layers.put({id: L.empty, rect: [rowX, px(H * 0.40), px(W * 0.86), px(48 * s)], order: [10, 2],
      paint: {kind: "text", text: "No games installed", font: "default", size: px(30 * s), color: [255, 255, 255, 255]}});
    layers.put({id: L.emptyHelp, rect: [rowX, px(H * 0.40 + 50 * s), px(W * 0.86), px(30 * s)], order: [10, 3],
      paint: {kind: "text", text: library.games_dir, font: "default", size: px(16 * s), color: [190, 196, 230, 220]}});
    return;
  }

  layers.put({id: L.glow, rect: [0, 0, 1, 1], order: [20, 0],
    paint: {kind: "image", asset: "glow.png"}});
  library.games.forEach((game, i) => {
    layers.put({id: L.cards + i, rect: [0, 0, cardW, cardH], order: [30, i],
      paint: {kind: "image", asset: `card-${i}.png`}});
    layers.put({id: L.initials + i, parent: L.cards + i, rect: [px(18 * s), px(cardH - 76 * s), cardW, px(64 * s)], order: [31, i],
      paint: {kind: "text", text: game.initials, font: "default", size: px(52 * s), color: [255, 255, 255, 235]}});
  });
  const hintY = px(H - 58 * s);
  layers.put({id: L.hintPill, rect: [px(W - 330 * s), hintY, px(290 * s), px(38 * s)], order: [50, 0],
    paint: {kind: "image", asset: "pill.png"}});
  layers.put({id: L.hintA, rect: [px(W - 318 * s), hintY + px(7 * s), px(24 * s), px(24 * s)], order: [50, 1],
    paint: {kind: "image", asset: "button.png"}});
  if (library.confirm !== "") {
    layers.put({id: L.hintA + 100, rect: [px(W - 312 * s), hintY + px(9 * s), px(20 * s), px(20 * s)], order: [50, 2],
      paint: {kind: "text", text: library.confirm, font: "default", size: px(14 * s), color: [20, 24, 40, 255]}});
  }
  layers.put({id: L.hintAText, rect: [px(W - 286 * s), hintY + px(8 * s), px(80 * s), px(24 * s)], order: [50, 2],
    paint: {kind: "text", text: "Play", font: "default", size: px(16 * s), color: [255, 255, 255, 235]}});
  layers.put({id: L.hintMove, rect: [px(W - 196 * s), hintY + px(7 * s), px(24 * s), px(24 * s)], order: [50, 3],
    paint: {kind: "image", asset: "dpad.png"}});
  layers.put({id: L.hintMove + 100, rect: [px(W - 164 * s), hintY + px(8 * s), px(120 * s), px(24 * s)], order: [50, 4],
    paint: {kind: "text", text: "Browse", font: "default", size: px(16 * s), color: [255, 255, 255, 200]}});
  select(0);
  layout(1);
}

function select(i) {
  const n = library.games.length;
  const s = library.scale;
  selected = Math.max(0, Math.min(n - 1, i));
  const game = library.games[selected];
  const top = rowY + cardH + px(34 * s);
  layers.put({id: L.title, rect: [rowX, top, px(W - 2 * rowX), px(46 * s)], order: [40, 0],
    paint: {kind: "text", text: game.name, font: "default", size: px(30 * s), color: [255, 255, 255, 255], layout: LINE}});
  layers.put({id: L.subtitle, rect: [rowX, top + px(46 * s), px(W * 0.5), px(26 * s)], order: [40, 1],
    paint: {kind: "text", text: `${selected + 1} / ${n}    ${game.folder}`, font: "default", size: px(15 * s), color: [170, 178, 220, 230], layout: LINE}});
}

// Ease the row toward the selection; the selected card grows and lifts.
function layout(t) {
  const n = library.games.length;
  const visible = Math.max(1, Math.floor((W - rowX) / (cardW + gap)) - 1);
  const target = Math.max(0, Math.min(Math.max(0, n - visible), selected - Math.floor(visible / 2)));
  scroll += (target - scroll) * t;
  focus += (selected - focus) * t;
  const step = cardW + gap;
  for (let i = 0; i < n; i++) {
    const near = Math.max(0, 1 - Math.abs(i - focus));
    const grow = 1 + 0.10 * near;
    const w = cardW * grow, h = cardH * grow;
    const x = rowX + (i - scroll) * step - (w - cardW) / 2;
    const y = rowY - (h - cardH) / 2 - 10 * library.scale * near;
    layers.put({id: L.cards + i, rect: [x, y, w, h], order: [30, i === selected ? 100 : i],
      opacity: px(150 + 105 * near), paint: {kind: "image", asset: `card-${i}.png`}});
  }
  const g = 18 * library.scale;
  const gx = rowX + (focus - scroll) * step - cardW * 0.05 - g;
  const gy = rowY - cardH * 0.05 - 10 * library.scale - g;
  layers.put({id: L.glow, rect: [gx, gy, cardW * 1.10 + 2 * g, cardH * 1.10 + 2 * g], order: [20, 0],
    paint: {kind: "image", asset: "glow.png"}});
}

function cardAt(x, y) {
  const step = cardW + gap;
  for (let i = 0; i < library.games.length; i++) {
    const cx = rowX + (i - scroll) * step;
    if (x >= cx && x < cx + cardW && y >= rowY && y < rowY + cardH) return i;
  }
  return -1;
}

function launch() {
  if (launching >= 0) return;
  launching = engine.timeMs;
  layers.put({id: L.fade, rect: [0, 0, W, H], order: [90, 0], opacity: 0,
    paint: {kind: "solid", color: [6, 8, 18, 255]}});
}

function onUpdate() {
  if (!library || library.games.length === 0) return;
  const now = engine.timeMs;
  const dt = Math.min(100, Math.max(0, now - lastTime));
  lastTime = now;
  if (launching >= 0) {
    const k = Math.min(1, (now - launching) / 260);
    layers.setOpacity(L.fade, px(255 * k));
    if (k >= 1) {
      fs.writeSave("launch.json", JSON.stringify({game: library.games[selected].id}));
      engine.quit();
    }
    return;
  }
  const input = engine.input;
  const pressed = input.buttonsPressed;
  const n = library.games.length;
  if (pressed & (LEFT | UP)) select(selected - 1);
  if (pressed & (RIGHT | DOWN)) select(selected + 1);
  if (pressed & PAGE_UP) select(selected - 5);
  if (pressed & PAGE_DOWN) select(selected + 5);
  if (pressed & ENTER) launch();
  if (pressed & MOUSE) {
    const [x, y] = input.pointer;
    const hit = cardAt(x, y);
    if (hit >= 0) {
      if (hit === selected) launch(); else select(hit);
    } else {
      const s = library.scale;
      const top = px(H - 58 * s);
      if (x >= px(W - 330 * s) && x < px(W - 206 * s) &&
          y >= top && y < top + px(38 * s)) launch();
    }
  }
  if (n > 0) layout(1 - Math.pow(0.0005, dt / 1000));
}
