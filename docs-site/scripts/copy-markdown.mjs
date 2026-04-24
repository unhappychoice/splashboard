// Mirror every page's Markdown source into `dist/` so each HTML page has a sibling
// `.md` at the same URL. Lets LLMs / other tools fetch the raw source without scraping
// HTML — same approach as the `llms.txt` family but per-page.
//
// `src/content/docs/guides/getting-started.md` → `dist/guides/getting-started.md`,
// served at `/guides/getting-started.md`.

import { readdir, mkdir, copyFile } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';

const SRC = 'src/content/docs';
const DIST = 'dist';

async function walk(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const out = [];
  for (const e of entries) {
    const p = join(dir, e.name);
    if (e.isDirectory()) {
      out.push(...(await walk(p)));
    } else if (e.name.endsWith('.md') || e.name.endsWith('.mdx')) {
      out.push(p);
    }
  }
  return out;
}

const files = await walk(SRC);
for (const src of files) {
  const rel = relative(SRC, src);
  const dest = join(DIST, rel);
  await mkdir(dirname(dest), { recursive: true });
  await copyFile(src, dest);
}
console.log(`copied ${files.length} markdown pages to ${DIST}/`);
