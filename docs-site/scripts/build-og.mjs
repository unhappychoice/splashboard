// Rasterise `public/og.svg` to `public/og.png` for social-card previews. SVG OG images
// work on some platforms (GitHub, Slack) but not others (Twitter/X, Facebook, LinkedIn
// render blank for SVG `og:image`), so ship a PNG alongside the source.

import { readFile, writeFile } from 'node:fs/promises';
import sharp from 'sharp';

const src = 'public/og.svg';
const dest = 'public/og.png';

const svg = await readFile(src);
await writeFile(
  dest,
  await sharp(svg, { density: 200 }).resize(1200, 630).png().toBuffer(),
);
console.log(`wrote ${dest}`);
