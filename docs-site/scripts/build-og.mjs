// Build the OG social card (`public/og.png`).
//
// librsvg (under sharp) doesn't parse SVG `@font-face` directives at all — it resolves
// font-family via the host's fontconfig only. Since JetBrains Mono isn't guaranteed on CI
// or contributor machines, we do text → path conversion up front: each text line becomes
// a `<path d=...>` carrying the glyph outlines, so rasterisation needs nothing beyond
// whatever sharp already ships.

import { readFile, writeFile } from 'node:fs/promises';
import sharp from 'sharp';
import wawoff from 'wawoff2';
import opentype from 'opentype.js';

// Title uses Orbitron 700 for wordmark impact; body copy stays on JetBrains Mono so the
// card still reads as "terminal tool" at a glance. Two font families, one path per line.
const FONT_SRC = {
  title: 'node_modules/@fontsource/orbitron/files/orbitron-latin-700-normal.woff2',
  body: 'node_modules/@fontsource/jetbrains-mono/files/jetbrains-mono-latin-400-normal.woff2',
};

// Each entry becomes one <path> in the card. x / y position glyph baselines on the
// 1200×630 canvas. Palette mirrors Theme::default() in src/theme/mod.rs.
const LINES = [
  { text: 'splashboard', x: 80, y: 260, size: 140, font: 'title', fill: '#ff8c7a' },
  { text: 'A customizable terminal splash —', x: 82, y: 360, size: 32, font: 'body', fill: '#c5d2dc' },
  { text: 'rendered on shell startup and on cd.', x: 82, y: 404, size: 32, font: 'body', fill: '#c5d2dc' },
  { text: 'per-directory configs · one splash per repo', x: 82, y: 520, size: 26, font: 'body', fill: '#7ea0b5' },
];

async function loadFont(path) {
  const ttf = Buffer.from(await wawoff.decompress(await readFile(path)));
  return opentype.parse(ttf.buffer.slice(ttf.byteOffset, ttf.byteOffset + ttf.byteLength));
}

const fonts = {
  title: await loadFont(FONT_SRC.title),
  body: await loadFont(FONT_SRC.body),
};

const paths = LINES.map((line) => {
  const glyphs = fonts[line.font].getPath(line.text, line.x, line.y, line.size);
  return `<path d="${glyphs.toPathData(2)}" fill="${line.fill}"/>`;
}).join('\n  ');

const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 630" shape-rendering="crispEdges">
  <rect width="1200" height="630" fill="#0e172a"/>
  <rect x="0" y="0" width="1200" height="72" fill="#17243b"/>
  <rect x="0" y="71" width="1200" height="1" fill="#4a6b75"/>
  <circle cx="48" cy="36" r="14" fill="#ff5f57"/>
  <circle cx="96" cy="36" r="14" fill="#febc2e"/>
  <circle cx="144" cy="36" r="14" fill="#28c840"/>
  ${paths}
  <g transform="translate(82, 560)">
    <rect width="18" height="18" fill="#0a1424"/>
    <rect x="22" width="18" height="18" fill="#0d3f4a"/>
    <rect x="44" width="18" height="18" fill="#2a8c8f"/>
    <rect x="66" width="18" height="18" fill="#0d3f4a"/>
    <rect x="88" width="18" height="18" fill="#2a8c8f"/>
    <rect x="110" width="18" height="18" fill="#ff8c7a"/>
    <rect x="132" width="18" height="18" fill="#ffc66b"/>
    <rect x="154" width="18" height="18" fill="#0d3f4a"/>
    <rect x="176" width="18" height="18" fill="#0a1424"/>
    <rect x="198" width="18" height="18" fill="#2a8c8f"/>
    <rect x="220" width="18" height="18" fill="#ff8c7a"/>
    <rect x="242" width="18" height="18" fill="#ffc66b"/>
  </g>
</svg>`;

await writeFile(
  'public/og.png',
  await sharp(Buffer.from(svg), { density: 200 }).resize(1200, 630).png().toBuffer(),
);
console.log('wrote public/og.png');
