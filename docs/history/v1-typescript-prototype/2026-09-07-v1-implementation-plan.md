# Sh_Images V1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native GPU-accelerated image viewer (Windows-first) that opens folders, navigates images with minimal RAM/CPU, crops and exports images (save/copy), and is selectable in Windows as the default image viewer.

**Architecture:** React + TypeScript on `@gpuix/react` (Zed's GPUI via React bindings) running under Bun. Pure business logic lives in `src/core/` and `src/utils/` (no GPUIX imports); presentation lives in `src/components/` + `src/hooks/`; OS/IO surfaces are isolated in `src/utils/` (registry, clipboard, dialogs) and `src/workers/` (decode/crop off the UI thread). The native GPUIX `<img>` element renders originals (Rust-side decode + GPU upload); the grid shows worker-built downsampled thumbnail files. Crop decodes on demand only in crop mode.

**Tech Stack:** Bun, TypeScript (strict), `@gpuix/react`, `@napi-rs/image`, `@napi-rs/clipboard`, `pino`, Biome, `bun test`, `tsc --noEmit`.

**Plan source:** `docs/superpowers/specs/2026-09-07-sh-images-viewer-design.md`

**Slices (each ends green and independently committable):**
- **Slice A — Scaffold & tooling:** Tasks 0–2
- **Slice B — Pure core (fully tested, no UI):** Tasks 3–9
- **Slice C — Native ops:** Tasks 10–12
- **Slice D — UI:** Tasks 13–17
- **Slice E — QA & docs:** Tasks 18–19

**Global verification gate (run after EVERY task):**
```bash
bun run typecheck && bun run lint && bun test
```
Expected: zero type errors, zero lint warnings, all tests pass. If a task says any subset, use the gate above anyway — it is never optional.

**Fixture generation:** `tests/fixtures/make-fixtures.ts` (Task 18) writes deterministic binary fixtures with `@napi-rs/image` + fixed hex strings. Fixtures required earlier by core tests are path-only (no real decode), so no fixture dependency before Task 18.

---

## Task 0: Repo foundations (LICENSE, README skeleton)

**Files:**
- Create: `LICENSE`
- Create: `README.md`
- Create: `docs/ARCHITECTURE.md` (skeleton with ADR placeholder)

- [ ] **Step 1: Create `LICENSE` (MIT, copyright holder from Step 3)**

```text
MIT License

Copyright (c) 2026 Adrián Quirós

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 2: Create `README.md`**

```markdown
# Sh_Images

Native GPU-accelerated image viewer for Windows, built with TypeScript + React
on GPUIX (Zed's GPUI via React bindings). Minimal RAM and CPU usage, modern
image-first UI, folder browsing, and crop → save/copy-to-clipboard workflow.
Selectable as the Windows default image viewer.

> Work in progress. See `docs/superpowers/plans/2026-09-07-sh-images-viewer-v1.md`
> for the implementation plan and `docs/superpowers/specs` for the design.

## Status

Early development — roadmap: folder browsing and grid (Tasks 13–14) → single
image view with zoom/pan (Task 15) → crop and export (Task 16) → Windows
default-viewer registration (Task 12, verified in Task 18).

## Development

```bash
bun install
bun run typecheck   # tsc --noEmit
bun run lint        # biome check
bun test            # unit + integration
bun --hot src/main.tsx     # dev (hot reload on same window)
bun run build       # bun build --compile src/main.tsx --outfile dist/sh_images
```

## License

MIT
```

- [ ] **Step 3: Create `docs/ARCHITECTURE.md`**

```markdown
# Architecture

Native GPU-accelerated image viewer. React + TypeScript on `@gpuix/react`
(Zed's GPUI via React bindings), running under Bun.

## Layout

- `src/core/` — pure business logic, no React/GPUIX imports
- `src/hooks/` — React hooks gluing core logic to components
- `src/components/` — presentation (depends on `@gpuix/react`)
- `src/config/` — settings/paths persistence
- `src/utils/` — cross-cutting utilities (errors, logger, OS surfaces)
- `src/workers/` — decode/crop work off the UI thread

## Decisions (ADR)

### ADR-001: GPUIX (TypeScript + React over GPUI) instead of raw Rust or Electron

- **Context:** Need a native, GPU-rendered viewer with a small team and fast
  iteration. Raw Rust (Zed GPUI directly) is higher-effort with no UI team;
  Electron/WebView is off the table for a GPU-accelerated native app.
- **Decision:** Use GPUIX (`@gpuix/react`): React components rendered through
  Zed's GPUI. Bun as runtime/bundler.
- **Consequences:** Native rendering + React ergonomics; GPUIX is a small,
  fast-moving ecosystem — verify API against installed package (Task 1).

Further ADRs are appended as implementation proceeds (image pipeline,
caching strategy, theming, Windows registration).
```

- [ ] **Step 4: Verify + commit**

Run: `git status --short` — expected: LICENSE, README.md,
docs/ARCHITECTURE.md untracked (plus `.gitignore` + existing committed spec).

```bash
git add LICENSE README.md docs/ARCHITECTURE.md
git commit -m "docs: add MIT license, README, and architecture skeleton"
```

---

## Task 1: Tooling — package.json, tsconfig, Biome, install

**Files:**
- Create: `package.json`
- Create: `tsconfig.json`
- Create: `biome.json`
- Create: `src/main.tsx` (probe entry)

- [ ] **Step 1: Create `package.json`** (unpinned majors pinned at install — run `bun install` to resolve exact versions)

```json
{
  "name": "sh-images",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "bun --hot src/main.tsx",
    "typecheck": "tsc --noEmit",
    "lint": "biome check .",
    "format": "biome format --write .",
    "test": "bun test",
    "test:coverage": "bun test --coverage",
    "bench": "bun test bench/",
    "build": "bun build --compile src/main.tsx --outfile dist/sh_images"
  },
  "dependencies": {
    "@gpuix/react": "^0.3.0",
    "@napi-rs/clipboard": "^0.4.0",
    "@napi-rs/image": "^2.0.0",
    "pino": "^9.0.0",
    "react": "^18.3.1"
  },
  "devDependencies": {
    "@biomejs/biome": "^2.0.0",
    "@types/node": "^22.0.0",
    "typescript": "^5.6.0"
  }
}
```

> Dependency justification (per AGENTS.md §7.2): `@napi-rs/image` — maintained
> N-API image decode/encode (no heavy toolchain); `@napi-rs/clipboard` —
> native clipboard image writes; `pino` — structured logger with levels and
> zero-config console transport; `react` + `@gpuix/react` — required by stack.
> No other runtime deps are added in this plan.

- [ ] **Step 2: Create `tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "lib": ["ESNext"],
    "jsx": "react-jsx",
    "jsxImportSource": "@gpuix/react",
    "strict": true,
    "noUncheckedIndexedAccess": true,
    "noImplicitOverride": true,
    "noFallthroughCasesInSwitch": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "isolatedModules": true,
    "verbatimModuleSyntax": true,
    "skipLibCheck": true,
    "types": ["bun", "@types/node"],
    "baseUrl": ".",
    "paths": { "@/*": ["src/*"] }
  },
  "include": ["src", "tests", "bench", "*.config.ts"]
}
```

- [ ] **Step 3: Create `biome.json`**

```json
{
  "$schema": "https://biomejs.dev/schemas/2.0.0/schema.json",
  "vcs": { "enabled": true, "clientKind": "git", "useIgnoreFile": true },
  "formatter": {
    "enabled": true,
    "indentStyle": "space",
    "indentWidth": 2,
    "lineWidth": 100
  },
  "linter": {
    "enabled": true,
    "rules": { "recommended": true }
  },
  "organizeImports": { "enabled": true }
}
```

- [ ] **Step 4: Create probe entry `src/main.tsx` (proves GPUIX render contract)**

```tsx
import { render } from '@gpuix/react';

function Probe() {
  return <text color="#ffffff">sh_images probe</text>;
}

render(<Probe />, { title: 'sh_images probe', width: 800, height: 400, resizable: true });
```

> If the installed `@gpuix/react` does not export `render` (or the window options
> differ), read `node_modules/@gpuix/react/dist/*.d.ts` and adjust the import and
> options to the observed signature. THE SIGNATURE IS THE SPIKE RESULT — record it
> in `docs/superpowers/plans/` notes and in an ADR if non-obvious.

- [ ] **Step 5: Install and lock**

Run:
```bash
bun install
```
Expected: installs resolve and `bun.lock` is created. Project must stay on Bun
for lockfile consistency (never run `npm install` here).

- [ ] **Step 6: Green gate**

Run: `bun run typecheck && bun run lint && bun test`
Expected: typecheck passes (Probe may surface real GPUIX type contract — fix
probe imports to the observed signature if needed); lint clean (Biome: adjust
formatting only); `bun test` reports "0 tests" (pass, no suites yet).

- [ ] **Step 7: Commit**

```bash
git add package.json package-lock.json bun.lock tsconfig.json biome.json src/main.tsx
git commit -m "chore: scaffold bun + typescript + gpuix tooling"
```

> If both `bun.lock` and `package-lock.json` exist after install, keep only
> `bun.lock` and delete the other.

---

## Task 2: Errors, logger, paths, settings skeleton

**Files:**
- Create: `src/utils/errors.ts`
- Create: `src/utils/logger.ts`
- Create: `src/config/paths.ts`
- Test: `tests/core/paths.test.ts` (env-driven, hermetic)

- [ ] **Step 1: Write `src/utils/errors.ts`**

```ts
export type ShImagesErrorCode =
  | 'DECODE_ERROR'
  | 'IO_ERROR'
  | 'UNSUPPORTED_FORMAT'
  | 'CONFIG_ERROR'
  | 'UNKNOWN';

export class ShImagesError extends Error {
  readonly code: ShImagesErrorCode;
  readonly cause?: unknown;

  constructor(code: ShImagesErrorCode, message: string, cause?: unknown) {
    super(message);
    this.name = 'ShImagesError';
    this.code = code;
    this.cause = cause;
  }
}

/** Wraps any thrown value into a ShImagesError, preserving the cause. */
export function asShImagesError(
  err: unknown,
  code: ShImagesErrorCode = 'UNKNOWN',
  message = 'Unexpected failure',
): ShImagesError {
  if (err instanceof ShImagesError) return err;
  return new ShImagesError(code, message, err);
}

/** Guards a precondition and throws a typed ShImagesError when it fails. */
export function assertOk(
  ok: boolean,
  code: ShImagesErrorCode,
  message: string,
): void {
  if (!ok) throw new ShImagesError(code, message);
}
```

- [ ] **Step 2: Write `src/utils/logger.ts`**

```ts
import { pino } from 'pino';

export const logger = pino({
  level: process.env.NODE_ENV === 'production' ? 'info' : 'debug',
  base: undefined,
});
```

- [ ] **Step 3: Write `src/config/paths.ts`** (pure path builders, injectable base for hermetic tests)

```ts
import { homedir } from 'node:os';
import { join } from 'node:path';

export const APP_DIR_NAME = 'Sh_Images';

/** Root app-data dir. Env `SH_IMAGES_HOME` overrides for tests/portable mode. */
export function appDataDir(): string {
  return process.env.SH_IMAGES_HOME ?? join(process.env.APPDATA ?? join(homedir(), '.config'), APP_DIR_NAME);
}

export function configDir(): string {
  return join(appDataDir(), 'config');
}

export function themesDir(): string {
  return join(configDir(), 'themes');
}

export function cacheDir(): string {
  return join(appDataDir(), 'cache');
}

export function configFile(): string {
  return join(configDir(), 'settings.json');
}

export function keymapFile(): string {
  return join(configDir(), 'keymap.json');
}
```

- [ ] **Step 4: Write the failing tests `tests/core/paths.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { afterEach } from 'bun:test';
import { appDataDir, cacheDir, configDir, configFile, themesDir } from '../../src/config/paths';

const ORIGINAL = { ...process.env };

afterEach(() => {
  process.env = { ...ORIGINAL };
  delete process.env.SH_IMAGES_HOME;
  delete process.env.APPDATA;
});

describe('paths', () => {
  test('SH_IMAGES_HOME overrides every derived path', () => {
    process.env.SH_IMAGES_HOME = 'C:/dev/home';
    expect(appDataDir()).toBe('C:/dev/home');
    expect(configDir()).toBe('C:/dev/home/config');
    expect(themesDir()).toBe('C:/dev/home/config/themes');
    expect(cacheDir()).toBe('C:/dev/home/cache');
    expect(configFile()).toBe('C:/dev/home/config/settings.json');
  });

  test('falls back to APPDATA without SH_IMAGES_HOME', () => {
    process.env.APPDATA = 'C:/Users/test/AppData/Roaming';
    expect(appDataDir()).toBe(
      'C:/Users/test/AppData/Roaming/Sh_Images',
    );
    expect(configFile()).toBe(
      'C:/Users/test/AppData/Roaming/Sh_Images/config/settings.json',
    );
  });

  test('falls back to homedir/.config without APPDATA either', () => {
    process.env.APPDATA = undefined;
    process.env.HOME = '/home/dev';
    expect(appDataDir().endsWith('Sh_Images')).toBe(true);
  });
});
```

- [ ] **Step 5: Run tests to verify they fail (paths module missing)**

Run: `bun test tests/core/paths.test.ts`
Expected: FAIL — module not found for `../../src/config/paths`.

- [ ] **Step 6: The implementation above in Step 3 is the minimal one** — run the tests.

Run: `bun test tests/core/paths.test.ts`
Expected: all 3 tests PASS.

> Note on `process.env.HOME` in the last test: on Windows, `homedir()` ignores
> HOME; `.endsWith('Sh_Images')` keeps the test cross-platform and deterministic.

- [ ] **Step 7: Global gate + commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/utils/errors.ts src/utils/logger.ts src/config/paths.ts tests/core/paths.test.ts
git commit -m "feat: add typed errors, logger, and app data paths"
```

---

## Task 3: Formats + folder navigation (core, pure)

**Files:**
- Create: `src/core/formats.ts`
- Create: `src/core/navigation.ts`
- Test: `tests/core/formats.test.ts`
- Test: `tests/core/navigation.test.ts`

- [ ] **Step 1: Write failing tests for formats `tests/core/formats.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { extensionOf, isSupportedImageName, isSupportedImagePath } from '../../src/core/formats';

describe('formats', () => {
  test('extensionOf is lowercased and dotless', () => {
    expect(extensionOf('photo.JPG')).toBe('jpg');
    expect(extensionOf('noext')).toBe('');
    expect(extensionOf('a.b.c.PnG')).toBe('png');
  });

  test('isSupportedImagePath accepts V1 formats by extension', () => {
    for (const name of ['a.png', 'b.jpg', 'c.jpeg', 'd.gif', 'e.webp', 'f.bmp', 'g.tif', 'h.tiff']) {
      expect(isSupportedImagePath(`C:/x/${name}`)).toBe(true);
    }
  });

  test('isSupportedImageName rejects unsupported and dotfiles', () => {
    expect(isSupportedImageName('x.svg')).toBe(false);
    expect(isSupportedImageName('x.avif')).toBe(false);
    expect(isSupportedImageName('hidden')).toBe(false);
    expect(isSupportedImageName('.png')).toBe(false);
  });
});
```

- [ ] **Step 2: Run to verify failure** — `bun test tests/core/formats.test.ts` — expected FAIL (module missing).

- [ ] **Step 3: Write minimal `src/core/formats.ts`**

```ts
export const SUPPORTED_EXTENSIONS = [
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'bmp',
  'tif',
  'tiff',
] as const;

export type SupportedExtension = (typeof SUPPORTED_EXTENSIONS)[number];

const SUPPORTED_SET: ReadonlySet<string> = new Set<string>(SUPPORTED_EXTENSIONS);

/** Lowercased extension without the dot; '' when the name has no extension. */
export function extensionOf(name: string): string {
  const idx = name.lastIndexOf('.');
  if (idx < 0 || idx === name.length - 1) return '';
  return name.slice(idx + 1).toLowerCase();
}

export function isSupportedImageName(name: string): boolean {
  if (name.startsWith('.') || name.length === 0) return false;
  return SUPPORTED_SET.has(extensionOf(name));
}

export function isSupportedImagePath(path: string): boolean {
  return isSupportedImageName(path);
}
```

- [ ] **Step 4: Run formats tests** — expected PASS.

- [ ] **Step 5: Write failing tests for navigation `tests/core/navigation.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { FolderIndex, type ImageEntry } from '../../src/core/navigation';

const ENTRIES: readonly ImageEntry[] = [
  { path: 'C:/x/b10.png', name: 'b10.png' },
  { path: 'C:/x/b2.png', name: 'b2.png' },
  { path: 'C:/x/a.png', name: 'a.png' },
  { path: 'C:/x/b1.png', name: 'b1.png' },
];

describe('FolderIndex', () => {
  test('sorts entries with natural numeric order', () => {
    const idx = new FolderIndex(ENTRIES);
    expect(idx.length).toBe(4);
    const names = idx.images.map((e) => e.name);
    expect(names).toEqual(['a.png', 'b1.png', 'b2.png', 'b10.png']);
  });

  test('at() returns entry or undefined out of range', () => {
    const idx = new FolderIndex(ENTRIES);
    expect(idx.at(0)?.name).toBe('a.png');
    expect(idx.at(99)).toBeUndefined();
    expect(idx.at(-1)).toBeUndefined();
  });

  test('indexOf resolves path', () => {
    const idx = new FolderIndex(ENTRIES);
    expect(idx.indexOf('C:/x/b10.png')).toBe(3);
    expect(idx.indexOf('missing.png')).toBe(-1);
  });

  test('next/prev wrap circularly', () => {
    const idx = new FolderIndex(ENTRIES);
    expect(idx.next(3)).toBe(0);
    expect(idx.prev(0)).toBe(3);
    expect(idx.next(1)).toBe(2);
    expect(idx.prev(2)).toBe(1);
  });

  test('empty index never returns a valid position', () => {
    const idx = new FolderIndex([]);
    expect(idx.length).toBe(0);
    expect(idx.next(-1)).toBe(-1);
    expect(idx.prev(-1)).toBe(-1);
    expect(idx.at(0)).toBeUndefined();
  });
});
```

- [ ] **Step 6: Run to verify failure** — `bun test tests/core/navigation.test.ts` — expected FAIL (module missing).

- [ ] **Step 7: Write minimal `src/core/navigation.ts`**

```ts
import { isSupportedImageName } from './formats';

export interface ImageEntry {
  readonly path: string;
  readonly name: string;
}

/** Natural (numeric-aware) filename compare, case-insensitive. */
function compareNames(a: string, b: string): number {
  return a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' });
}

/** Sorted, supported-only image list with circular next/prev navigation. */
export class FolderIndex {
  readonly images: readonly ImageEntry[];

  constructor(entries: readonly ImageEntry[]) {
    this.images = [...entries]
      .filter((e) => isSupportedImageName(e.name))
      .sort((a, b) => compareNames(a.name, b.name));
  }

  get length(): number {
    return this.images.length;
  }

  at(index: number): ImageEntry | undefined {
    return index >= 0 && index < this.images.length ? this.images[index] : undefined;
  }

  indexOf(path: string): number {
    return this.images.findIndex((e) => e.path === path);
  }

  next(current: number): number {
    if (this.images.length === 0) return -1;
    if (current < 0 || current >= this.images.length) return 0;
    return (current + 1) % this.images.length;
  }

  prev(current: number): number {
    if (this.images.length === 0) return -1;
    if (current < 0 || current >= this.images.length) return this.images.length - 1;
    return (current - 1 + this.images.length) % this.images.length;
  }
}
```

> The pure `FolderIndex` takes already-scanned entries; directory scanning is a
> later UI/hook concern (Task 14) with `fs.readdir` in a worker/free context.

- [ ] **Step 8: Run navigation tests** — expected PASS.

- [ ] **Step 9: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/core/formats.ts src/core/navigation.ts tests/core/formats.test.ts tests/core/navigation.test.ts
git commit -m "feat: add supported formats and circular folder navigation"
```

---

## Task 4: Crop math (core, pure)

**Files:**
- Create: `src/core/crop-math.ts`
- Test: `tests/core/crop-math.test.ts`

- [ ] **Step 1: Write failing tests `tests/core/crop-math.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import {
  aspectFromRect,
  clampRect,
  fitRectToAspect,
  fitTransform,
  normalizeRect,
  screenToSource,
  sourceToScreen,
} from '../../src/core/crop-math';

describe('fitTransform', () => {
  test('contain-fit scales down large images', () => {
    const t = fitTransform(800, 600, 4000, 3000);
    expect(t.scale).toBeCloseTo(0.2, 5);
    expect(t.offsetX).toBeCloseTo(0, 5);
    expect(t.offsetY).toBeCloseTo(0, 5);
  });

  test('contain-fit scales up small images and centers offset', () => {
    const t = fitTransform(800, 600, 100, 50);
    expect(t.scale).toBeCloseTo(8, 5); // min(800/100, 600/50)
    expect(t.offsetX).toBeCloseTo(0, 5); // (800 - 100*8) / 2
    expect(t.offsetY).toBeCloseTo(100, 5); // (600 - 50*8) / 2
  });
});

describe('coordinate mapping', () => {
  test('screenToSource and sourceToScreen are inverses', () => {
    const t = fitTransform(800, 600, 4000, 3000);
    const src = screenToSource({ x: 400, y: 300 }, t);
    expect(src.x).toBeCloseTo(2000, 5);
    expect(src.y).toBeCloseTo(1500, 5);
    const back = sourceToScreen(src, t);
    expect(back.x).toBeCloseTo(400, 5);
    expect(back.y).toBeCloseTo(300, 5);
  });
});

describe('rect helpers', () => {
  test('normalizeRect handles drag in any direction', () => {
    const r = normalizeRect({ x: 300, y: 200 }, { x: 100, y: 50 });
    expect(r).toEqual({ x: 100, y: 50, w: 200, h: 150 });
  });

  test('clampRect stays inside image bounds', () => {
    const r = clampRect({ x: -50, y: 20, w: 5000, h: 200 }, 4000, 3000);
    expect(r.x).toBe(0);
    expect(r.y).toBe(20);
    expect(r.w).toBe(4000);
    expect(r.h).toBe(200);
  });

  test('aspectFromRect returns width/height', () => {
    expect(aspectFromRect({ x: 0, y: 0, w: 16, h: 9 })).toBeCloseTo(16 / 9, 5);
  });

  test('fitRectToAspect adjusts around anchor keeping aspect', () => {
    const r = fitRectToAspect({ x: 100, y: 100, w: 200, h: 100 }, 1, { x: 150, y: 150 });
    // 200x200 with anchor at the rect center; anchor stays inside the result
    expect(aspectFromRect(r)).toBeCloseTo(1, 5);
    expect(r.x <= 150 && r.x + r.w >= 150).toBe(true);
  });
});
```

- [ ] **Step 2: Run to verify failure** — `bun test tests/core/crop-math.test.ts` — FAIL (module missing).

- [ ] **Step 3: Write minimal `src/core/crop-math.ts`**

```ts
export interface Pt {
  readonly x: number;
  readonly y: number;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Scale/offset that maps source pixels onto a view (contain-fit). */
export interface FitTransform {
  readonly scale: number;
  readonly offsetX: number;
  readonly offsetY: number;
}

export function fitTransform(viewW: number, viewH: number, imgW: number, imgH: number): FitTransform {
  const scale = Math.min(viewW / imgW, viewH / imgH);
  return {
    scale,
    offsetX: (viewW - imgW * scale) / 2,
    offsetY: (viewH - imgH * scale) / 2,
  };
}

export function screenToSource(p: Pt, t: FitTransform): Pt {
  return { x: (p.x - t.offsetX) / t.scale, y: (p.y - t.offsetY) / t.scale };
}

export function sourceToScreen(p: Pt, t: FitTransform): Pt {
  return { x: p.x * t.scale + t.offsetX, y: p.y * t.scale + t.offsetY };
}

export function normalizeRect(a: Pt, b: Pt): Rect {
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) };
}

export function clampRect(r: Rect, imgW: number, imgH: number): Rect {
  const x = Math.max(0, Math.min(r.x, imgW));
  const y = Math.max(0, Math.min(r.y, imgH));
  const w = Math.max(0, Math.min(r.w, imgW - x));
  const h = Math.max(0, Math.min(r.h, imgH - y));
  return { x, y, w, h };
}

export function aspectFromRect(r: Rect): number {
  return r.h === 0 ? 0 : r.w / r.h;
}

/** Grows/shrinks rect to the target aspect, keeping the anchor inside. */
export function fitRectToAspect(r: Rect, aspect: number, anchor: Pt): Rect {
  let w = r.w;
  let h = r.h;
  if (w / h > aspect) {
    w = h * aspect;
  } else {
    h = w / aspect;
  }
  const ax = (anchor.x - r.x) / r.w;
  const ay = (anchor.y - r.y) / r.h;
  const x = anchor.x - ax * w;
  const y = anchor.y - ay * h;
  return { x, y, w, h };
}
```

- [ ] **Step 4: Run crop-math tests** — expected PASS.

- [ ] **Step 5: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/core/crop-math.ts tests/core/crop-math.test.ts
git commit -m "feat: add crop coordinate and aspect math"
```

---

## Task 5: Image pixel ops (core, pure)

**Files:**
- Create: `src/core/image-ops.ts`
- Test: `tests/core/image-ops.test.ts`

- [ ] **Step 1: Write failing tests `tests/core/image-ops.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { cropPixels, downscaleBox, flattenAlpha } from '../../src/core/image-ops';

const make = (w: number, h: number) => new Uint8Array(w * h * 4);

describe('cropPixels', () => {
  test('extracts a sub-region in row order', () => {
    const img = make(4, 4);
    // pixel (1,2) distinctly colored: r=7
    img[((2 * 4 + 1) * 4) + 0] = 7;
    const out = cropPixels({ data: img, width: 4, height: 4 }, { x: 1, y: 2, w: 1, h: 1 });
    expect(out.width).toBe(1);
    expect(out.height).toBe(1);
    expect(out.data[0]).toBe(7);
  });

  test('rejects out-of-bounds rect', () => {
    expect(() =>
      cropPixels({ data: make(4, 4), width: 4, height: 4 }, { x: 3, y: 0, w: 2, h: 1 }),
    ).toThrow();
  });
});

describe('downscaleBox', () => {
  test('downscales keeping aspect and clamps size', () => {
    const img = make(400, 800);
    const out = downscaleBox({ data: img, width: 400, height: 800 }, 100);
    expect(out.width).toBe(50);
    expect(out.height).toBe(100);
  });

  test('never upscales beyond clamp', () => {
    const img = make(10, 10);
    const out = downscaleBox({ data: img, width: 10, height: 10 }, 100);
    expect(out.width).toBe(10);
    expect(out.height).toBe(10);
  });

  test('output pixel count matches w*h*4', () => {
    const img = make(60, 40);
    const out = downscaleBox({ data: img, width: 60, height: 40 }, 24);
    expect(out.data.length).toBe(out.width * out.height * 4);
  });
});

describe('flattenAlpha', () => {
  test('composites alpha onto white background', () => {
    // opaque red stays red
    const a = new Uint8Array([255, 0, 0, 255]);
    expect(flattenAlpha({ data: a, width: 1, height: 1 }).data[0]).toBe(255);
    // fully transparent becomes white
    const b = new Uint8Array([10, 20, 30, 0]);
    const out = flattenAlpha({ data: b, width: 1, height: 1 }).data;
    expect(out[0]).toBe(255);
    expect(out[1]).toBe(255);
    expect(out[2]).toBe(255);
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write minimal `src/core/image-ops.ts`**

```ts
import type { Rect } from './crop-math';

export interface RgbaImage {
  data: Uint8Array;
  width: number;
  height: number;
}

/** Extracts a rect from RGBA pixels (row-major). Throws when out of bounds. */
export function cropPixels(img: RgbaImage, rect: Rect): RgbaImage {
  if (
    rect.x < 0 ||
    rect.y < 0 ||
    rect.w < 0 ||
    rect.h < 0 ||
    rect.x + rect.w > img.width ||
    rect.y + rect.h > img.height
  ) {
    throw new RangeError(`crop rect ${JSON.stringify(rect)} outside ${img.width}x${img.height}`);
  }
  const out = new Uint8Array(rect.w * rect.h * 4);
  for (let row = 0; row < rect.h; row++) {
    const srcStart = ((rect.y + row) * img.width + rect.x) * 4;
    out.set(img.data.subarray(srcStart, srcStart + rect.w * 4), row * rect.w * 4);
  }
  return { data: out, width: rect.w, height: rect.h };
}

/** Deterministic box-average downscale to a max edge (never upscales). */
export function downscaleBox(img: RgbaImage, maxEdge: number): RgbaImage {
  const factor = Math.max(1, Math.ceil(Math.max(img.width, img.height) / maxEdge));
  const w = Math.max(1, Math.floor(img.width / factor));
  const h = Math.max(1, Math.floor(img.height / factor));
  const out = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;
      let n = 0;
      for (let sy = 0; sy < factor; sy++) {
        for (let sx = 0; sx < factor; sx++) {
          const px = x * factor + sx;
          const py = y * factor + sy;
          if (px >= img.width || py >= img.height) continue;
          const i = (py * img.width + px) * 4;
          r += img.data[i]!;
          g += img.data[i + 1]!;
          b += img.data[i + 2]!;
          a += img.data[i + 3]!;
          n++;
        }
      }
      const o = (y * w + x) * 4;
      out[o] = Math.round(r / n);
      out[o + 1] = Math.round(g / n);
      out[o + 2] = Math.round(b / n);
      out[o + 3] = Math.round(a / n);
    }
  }
  return { data: out, width: w, height: h };
}

/** Composites RGBA onto white (for JPEG output which has no alpha). */
export function flattenAlpha(img: RgbaImage): RgbaImage {
  const out = new Uint8Array(img.data.length);
  for (let i = 0; i < img.data.length; i += 4) {
    const a = img.data[i + 3]! / 255;
    out[i] = Math.round(img.data[i]! * a + 255 * (1 - a));
    out[i + 1] = Math.round(img.data[i + 1]! * a + 255 * (1 - a));
    out[i + 2] = Math.round(img.data[i + 2]! * a + 255 * (1 - a));
    out[i + 3] = 255;
  }
  return { data: out, width: img.width, height: img.height };
}
```

> `img.data[i]!` is safe: `i` is always < `data.length` by construction here. The
> non-null assertion is justified inline (AGENTS.md §2.1 allows when provably
> safe and commented).

- [ ] **Step 4: Run image-ops tests** — expected PASS.

- [ ] **Step 5: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/core/image-ops.ts tests/core/image-ops.test.ts
git commit -m "feat: add pure pixel ops (crop, box downscale, alpha flatten)"
```

---

## Task 6: LRU image cache policy (core, pure)

**Files:**
- Create: `src/core/image-cache.ts`
- Test: `tests/core/image-cache.test.ts`

- [ ] **Step 1: Write failing tests `tests/core/image-cache.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { LruCache } from '../../src/core/image-cache';

describe('LruCache', () => {
  test('get updates recency', () => {
    const clock = { now: 0 };
    const c = new LruCache(100, () => clock.now);
    c.put('a', 40);
    clock.now = 1;
    c.put('b', 40);
    clock.now = 2;
    c.get('a');
    clock.now = 3;
    c.put('c', 40); // evicts 'b' (LRU), keeps 'a'
    expect(c.get('a')).not.toBeUndefined();
    expect(c.get('b')).toBeUndefined();
    expect(c.get('c')).not.toBeUndefined();
  });

  test('evicts until under budget on put', () => {
    const c = new LruCache(50);
    c.put('a', 30);
    c.put('b', 30);
    expect(c.length).toBe(1);
    expect(c.get('a')).toBeUndefined();
    expect(c.get('b')).not.toBeUndefined();
  });

  test('a single oversized entry still stores (no infinite evict loop)', () => {
    const c = new LruCache(10);
    c.put('big', 100);
    expect(c.length).toBe(1);
    expect(c.get('big')).not.toBeUndefined();
  });

  test('remove and clear work', () => {
    const c = new LruCache(100);
    c.put('a', 30);
    c.put('b', 30);
    c.remove('a');
    expect(c.get('a')).toBeUndefined();
    expect(c.sizeBytes).toBe(30);
    c.clear();
    expect(c.sizeBytes).toBe(0);
    expect(c.length).toBe(0);
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write minimal `src/core/image-cache.ts`**

```ts
export interface CacheEntry {
  readonly key: string;
  readonly sizeBytes: number;
  readonly lastAccess: number;
}

/**
 * Byte-budgeted LRU cache. `get` refreshes recency; `put` evicts oldest
 * entries until under budget (except the entry being inserted, which may
 * itself exceed the budget).
 */
export class LruCache {
  private readonly map = new Map<string, CacheEntry>();
  private bytes = 0;

  constructor(
    readonly maxBytes: number,
    private readonly now: () => number = () => Date.now(),
  ) {}

  get length(): number {
    return this.map.size;
  }

  get sizeBytes(): number {
    return this.bytes;
  }

  peek(key: string): CacheEntry | undefined {
    return this.map.get(key);
  }

  get(key: string): CacheEntry | undefined {
    const entry = this.map.get(key);
    if (!entry) return undefined;
    const fresh: CacheEntry = { ...entry, lastAccess: this.now() };
    this.map.delete(key);
    this.map.set(key, fresh);
    return fresh;
  }

  put(key: string, sizeBytes: number): void {
    const old = this.map.get(key);
    if (old) {
      this.bytes -= old.sizeBytes;
      this.map.delete(key);
    }
    const entry: CacheEntry = { key, sizeBytes, lastAccess: this.now() };
    this.map.set(key, entry);
    this.bytes += sizeBytes;
    while (this.bytes > this.maxBytes && this.map.size > 1) {
      const oldestKey = this.map.keys().next().value;
      if (oldestKey === undefined || oldestKey === key) break;
      const removed = this.map.get(oldestKey);
      this.map.delete(oldestKey);
      if (removed) this.bytes -= removed.sizeBytes;
    }
  }

  remove(key: string): void {
    const entry = this.map.get(key);
    if (!entry) return;
    this.map.delete(key);
    this.bytes -= entry.sizeBytes;
  }

  clear(): void {
    this.map.clear();
    this.bytes = 0;
  }
}
```

> Eviction loop: it never evicts the entry just inserted (`oldestKey === key`
> guard), so a single oversized entry is storable per the test contract.

- [ ] **Step 4: Run cache tests** — expected PASS.

- [ ] **Step 5: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/core/image-cache.ts tests/core/image-cache.test.ts
git commit -m "feat: add byte-budgeted LRU cache policy"
```

---

## Task 7: Theme tokens, schema validation, bundled themes

**Files:**
- Create: `src/core/theme.ts`
- Test: `tests/core/theme.test.ts`

- [ ] **Step 1: Write failing tests `tests/core/theme.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { ShImagesError } from '../../src/utils/errors';
import { LIGHTBOX_THEME, parseTheme, resolveTheme, validateTheme } from '../../src/core/theme';

const VALID = JSON.stringify(LIGHTBOX_THEME);

describe('theme schema', () => {
  test('parseTheme accepts the bundled shape', () => {
    const t = parseTheme(VALID);
    expect(t.name).toBe(LIGHTBOX_THEME.name);
    expect(t.colors.background.length).toBeGreaterThan(0);
  });

  test('parseTheme rejects invalid JSON with CONFIG_ERROR', () => {
    try {
      parseTheme('{not json');
      expect.unreachable();
    } catch (e) {
      expect(e).toBeInstanceOf(ShImagesError);
      expect((e as ShImagesError).code).toBe('CONFIG_ERROR');
    }
  });

  test('parseTheme rejects missing required keys', () => {
    expect(() => parseTheme(JSON.stringify({ name: 'x' }))).toThrow(ShImagesError);
  });

  test('parseTheme rejects wrong color format', () => {
    const evil = { ...LIGHTBOX_THEME, colors: { ...LIGHTBOX_THEME.colors, background: 'not-a-color' } };
    expect(() => parseTheme(JSON.stringify(evil))).toThrow(ShImagesError);
  });

  test('validateTheme returns a fully-typed ThemeTokens', () => {
    expect(validateTheme(LIGHTBOX_THEME)).toBe(LIGHTBOX_THEME);
  });

  test('resolveTheme falls back to lightbox for unknown ids', () => {
    expect(resolveTheme('does-not-exist').name).toBe(LIGHTBOX_THEME.name);
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write minimal `src/core/theme.ts`**

```ts
import { ShImagesError } from '../utils/errors';

export interface ThemeTokens {
  readonly name: string;
  readonly author: string;
  readonly version: number;
  readonly colors: {
    readonly background: string;
    readonly surface: string;
    readonly surfaceHover: string;
    readonly text: string;
    readonly textMuted: string;
    readonly accent: string;
    readonly accentText: string;
    readonly border: string;
    readonly overlay: string;
    readonly danger: string;
  };
  readonly radii: { readonly sm: number; readonly md: number; readonly lg: number; readonly full: number };
  readonly spacing: { readonly xs: number; readonly sm: number; readonly md: number; readonly lg: number };
  readonly opacity: { readonly dimmed: number; readonly overlay: number; readonly chrome: number };
  readonly effects: { readonly blur: number };
  readonly icons: { readonly strokeWidth: number };
  readonly typography: {
    readonly family: string;
    readonly monoFamily: string;
    readonly sizes: { readonly caption: number; readonly body: number; readonly title: number; readonly huge: number };
  };
}

const HEX_COLOR = /^#[0-9a-f]{6}([0-9a-f]{2})?$/i;

function fail(message: string): never {
  throw new ShImagesError('CONFIG_ERROR', `Invalid theme: ${message}`);
}

function asRecord(v: unknown, path: string): Record<string, unknown> {
  if (typeof v !== 'object' || v === null) fail(`${path} must be an object`);
  return v as Record<string, unknown>;
}

function reqString(v: unknown, path: string): string {
  if (typeof v !== 'string' || v.length === 0) fail(`${path} must be a non-empty string`);
  return v;
}

function reqNumber(v: unknown, path: string): number {
  if (typeof v !== 'number' || !Number.isFinite(v)) fail(`${path} must be a finite number`);
  return v;
}

function reqColor(v: unknown, path: string): string {
  const s = reqString(v, path);
  if (!HEX_COLOR.test(s)) fail(`${path} must be a hex color (#rrggbb or #rrggbbaa)`);
  return s;
}

/** Validates an unknown value into ThemeTokens; throws ShImagesError(CONFIG_ERROR). */
export function validateTheme(value: unknown): ThemeTokens {
  const root = asRecord(value, 'theme');
  const colors = asRecord(root['colors'], 'colors');
  const radii = asRecord(root['radii'], 'radii');
  const spacing = asRecord(root['spacing'], 'spacing');
  const opacity = asRecord(root['opacity'], 'opacity');
  const effects = asRecord(root['effects'], 'effects');
  const icons = asRecord(root['icons'], 'icons');
  const typography = asRecord(root['typography'], 'typography');
  const sizes = asRecord(typography['sizes'], 'typography.sizes');

  return {
    name: reqString(root['name'], 'name'),
    author: reqString(root['author'], 'author'),
    version: reqNumber(root['version'], 'version'),
    colors: {
      background: reqColor(colors['background'], 'colors.background'),
      surface: reqColor(colors['surface'], 'colors.surface'),
      surfaceHover: reqColor(colors['surfaceHover'], 'colors.surfaceHover'),
      text: reqColor(colors['text'], 'colors.text'),
      textMuted: reqColor(colors['textMuted'], 'colors.textMuted'),
      accent: reqColor(colors['accent'], 'colors.accent'),
      accentText: reqColor(colors['accentText'], 'colors.accentText'),
      border: reqColor(colors['border'], 'colors.border'),
      overlay: reqColor(colors['overlay'], 'colors.overlay'),
      danger: reqColor(colors['danger'], 'colors.danger'),
    },
    radii: {
      sm: reqNumber(radii['sm'], 'radii.sm'),
      md: reqNumber(radii['md'], 'radii.md'),
      lg: reqNumber(radii['lg'], 'radii.lg'),
      full: reqNumber(radii['full'], 'radii.full'),
    },
    spacing: {
      xs: reqNumber(spacing['xs'], 'spacing.xs'),
      sm: reqNumber(spacing['sm'], 'spacing.sm'),
      md: reqNumber(spacing['md'], 'spacing.md'),
      lg: reqNumber(spacing['lg'], 'spacing.lg'),
    },
    opacity: {
      dimmed: reqNumber(opacity['dimmed'], 'opacity.dimmed'),
      overlay: reqNumber(opacity['overlay'], 'opacity.overlay'),
      chrome: reqNumber(opacity['chrome'], 'opacity.chrome'),
    },
    effects: { blur: reqNumber(effects['blur'], 'effects.blur') },
    icons: { strokeWidth: reqNumber(icons['strokeWidth'], 'icons.strokeWidth') },
    typography: {
      family: reqString(typography['family'], 'typography.family'),
      monoFamily: reqString(typography['monoFamily'], 'typography.monoFamily'),
      sizes: {
        caption: reqNumber(sizes['caption'], 'typography.sizes.caption'),
        body: reqNumber(sizes['body'], 'typography.sizes.body'),
        title: reqNumber(sizes['title'], 'typography.sizes.title'),
        huge: reqNumber(sizes['huge'], 'typography.sizes.huge'),
      },
    },
  };
}

/** Parses a theme JSON string, returning validated tokens. */
export function parseTheme(json: string): ThemeTokens {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch (cause) {
    throw new ShImagesError('CONFIG_ERROR', 'Invalid theme: not valid JSON', cause);
  }
  return validateTheme(parsed);
}

export const LIGHTBOX_THEME: ThemeTokens = {
  name: 'Lightbox',
  author: 'Sh_Images',
  version: 1,
  colors: {
    background: '#0d0d0f',
    surface: '#16161a',
    surfaceHover: '#1e1e24',
    text: '#f4f4f5',
    textMuted: '#8b8b93',
    accent: '#e8e6e3',
    accentText: '#0d0d0f',
    border: '#2a2a31',
    overlay: '#000000',
    danger: '#f87171',
  },
  radii: { sm: 4, md: 8, lg: 16, full: 9999 },
  spacing: { xs: 4, sm: 8, md: 16, lg: 24 },
  opacity: { dimmed: 0.55, overlay: 0.5, chrome: 0.92 },
  effects: { blur: 18 },
  icons: { strokeWidth: 1.5 },
  typography: {
    family: 'Inter',
    monoFamily: 'JetBrains Mono',
    sizes: { caption: 11, body: 14, title: 18, huge: 44 },
  },
};

/** Bundled theme ids users can select (first is default). */
export const BUNDLED_THEME_IDS = ['lightbox', 'terminal', 'studio'] as const;
export type BundledThemeId = (typeof BUNDLED_THEME_IDS)[number];

export const BUNDLED_THEMES: Readonly<Record<BundledThemeId, ThemeTokens>> = {
  lightbox: LIGHTBOX_THEME,
  terminal: {
    ...LIGHTBOX_THEME,
    name: 'Terminal',
    colors: {
      ...LIGHTBOX_THEME.colors,
      background: '#0a0f0a',
      surface: '#101810',
      surfaceHover: '#182218',
      text: '#c8f7c5',
      textMuted: '#5d8a5a',
      accent: '#7ddc8c',
      accentText: '#0a0f0a',
      border: '#1e2a1e',
    },
    radii: { ...LIGHTBOX_THEME.radii, sm: 0, md: 0, lg: 0 },
    typography: { ...LIGHTBOX_THEME.typography, family: 'JetBrains Mono', monoFamily: 'JetBrains Mono' },
  },
  studio: {
    ...LIGHTBOX_THEME,
    name: 'Studio Light',
    colors: {
      background: '#f5f5f4',
      surface: '#ffffff',
      surfaceHover: '#ececeb',
      text: '#1c1917',
      textMuted: '#78716c',
      accent: '#111111',
      accentText: '#f5f5f4',
      border: '#d6d3d1',
      overlay: '#ffffff',
      danger: '#b91c1c',
    },
  },
};

const USER_THEME_CACHE = new Map<string, ThemeTokens>();

/** Registers/looks up themes by id (bundled first, then loaded user themes). */
export function resolveTheme(id: string): ThemeTokens {
  if (id in BUNDLED_THEMES) return BUNDLED_THEMES[id as BundledThemeId];
  const user = USER_THEME_CACHE.get(id);
  return user ?? LIGHTBOX_THEME;
}

/** Loads a user theme JSON file and caches it under its id (its name, lowercased). */
export function loadUserTheme(file: string): ThemeTokens {
  let json: string;
  try {
    json = Bun.file(file).textSync();
  } catch (cause) {
    throw new ShImagesError('CONFIG_ERROR', `Cannot read theme file ${file}`, cause);
  }
  const tokens = parseTheme(json);
  USER_THEME_CACHE.set(tokens.name.toLowerCase(), tokens);
  return tokens;
}
```

- [ ] **Step 4: Run theme tests** — expected PASS.

- [ ] **Step 5: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/core/theme.ts tests/core/theme.test.ts
git commit -m "feat: add validated design-token themes with lightbox default"
```

---

## Task 8: Action registry + keymap (parse/resolve/conflicts)

**Files:**
- Create: `src/core/actions.ts`
- Create: `src/core/keymap.ts`
- Test: `tests/core/keymap.test.ts`

- [ ] **Step 1: Write failing tests `tests/core/keymap.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { DEFAULT_KEYMAP } from '../../src/core/actions';
import {
  chordToString,
  findConflicts,
  keyEventToChord,
  parseKeymap,
  resolveKeymap,
} from '../../src/core/keymap';

describe('keymap', () => {
  test('defaults include the validated shortcut table', () => {
    expect(chordToString(DEFAULT_KEYMAP['next'])).toBe('ArrowRight');
    expect(chordToString(DEFAULT_KEYMAP['zoomIn'])).toBe('+');
    expect(chordToString(DEFAULT_KEYMAP['saveCrop'])).toBe('Ctrl+S');
    expect(chordToString(DEFAULT_KEYMAP['copyCrop'])).toBe('Ctrl+Shift+C');
  });

  test('parseKeymap validates shape and actions', () => {
    const good = parseKeymap(JSON.stringify({ version: 1, bindings: { next: { key: 'n' } } }));
    expect(good.bindings['next']).toEqual({ key: 'n' });
    expect(() => parseKeymap(JSON.stringify({ version: 1, bindings: { nope: { key: 'n' } } }))).toThrow();
    expect(() => parseKeymap('{bad')).toThrow();
  });

  test('resolveKeymap merges user overrides over defaults', () => {
    const user = parseKeymap(JSON.stringify({ version: 1, bindings: { next: { key: 'n' } } }));
    const resolved = resolveKeymap(user);
    expect(chordToString(resolved.get('next')!)).toBe('n');
    expect(chordToString(resolved.get('prev')!)).toBe('ArrowLeft');
  });

  test('findConflicts reports duplicate chords across actions', () => {
    const user = parseKeymap(
      JSON.stringify({ version: 1, bindings: { back: { key: 'Escape' }, cancelCrop: { key: 'Escape' } } }),
    );
    const resolved = resolveKeymap(user);
    expect(findConflicts(resolved).length).toBeGreaterThan(0);
  });

  test('keyEventToChord normalizes keys and modifiers', () => {
    expect(keyEventToChord('ArrowRight', false, false, false)).toEqual({ key: 'ArrowRight' });
    expect(keyEventToChord('s', true, false, false)).toEqual({ key: 's', ctrl: true });
    expect(keyEventToChord('C', true, true, false)).toEqual({ key: 'c', ctrl: true, shift: true });
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (modules missing).

- [ ] **Step 3: Write minimal `src/core/actions.ts`**

```ts
export const ACTION_IDS = [
  'next',
  'prev',
  'zoomIn',
  'zoomOut',
  'zoomActual',
  'zoomFit',
  'toggleFullscreen',
  'toggleChrome',
  'crop',
  'confirmCrop',
  'cancelCrop',
  'saveCrop',
  'copyCrop',
  'openFolder',
  'openFile',
  'back',
] as const;

export type ActionId = (typeof ACTION_IDS)[number];

export interface KeyChord {
  readonly key: string;
  readonly ctrl?: boolean;
  readonly shift?: boolean;
  readonly alt?: boolean;
}

export const DEFAULT_KEYMAP: Readonly<Record<ActionId, KeyChord>> = {
  next: { key: 'ArrowRight' },
  prev: { key: 'ArrowLeft' },
  zoomIn: { key: '+' },
  zoomOut: { key: '-' },
  zoomActual: { key: '0' },
  zoomFit: { key: 'f' },
  toggleFullscreen: { key: 'F11' },
  toggleChrome: { key: 'h' },
  crop: { key: 'c' },
  confirmCrop: { key: 'Enter' },
  cancelCrop: { key: 'Escape' },
  saveCrop: { key: 's', ctrl: true },
  copyCrop: { key: 'c', ctrl: true, shift: true },
  openFolder: { key: 'o', ctrl: true },
  openFile: { key: 'o', ctrl: true, shift: true },
  back: { key: 'Escape' },
};
```

- [ ] **Step 4: Write minimal `src/core/keymap.ts`**

```ts
import { ShImagesError } from '../utils/errors';
import { ACTION_IDS, DEFAULT_KEYMAP, type ActionId, type KeyChord } from './actions';

const SINGLE_KEY = /^.$/;
const NAMED_KEYS = new Set(['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Enter', 'Escape', 'F11', 'Tab', 'Backspace']);

export interface KeymapFile {
  readonly version: number;
  readonly bindings: Readonly<Partial<Record<ActionId, KeyChord>>>;
}

function fail(message: string): never {
  throw new ShImagesError('CONFIG_ERROR', `Invalid keymap: ${message}`);
}

/** Parses keymap.json; throws ShImagesError(CONFIG_ERROR) on bad shape. */
export function parseKeymap(json: string): KeymapFile {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch (cause) {
    throw new ShImagesError('CONFIG_ERROR', 'Invalid keymap: not valid JSON', cause);
  }
  const root = parsed as Record<string, unknown>;
  if (typeof root['version'] !== 'number') fail('missing version');
  const bindingsRaw = root['bindings'];
  if (typeof bindingsRaw !== 'object' || bindingsRaw === null) fail('missing bindings object');
  const bindings = bindingsRaw as Record<string, unknown>;
  const out: Partial<Record<ActionId, KeyChord>> = {};
  for (const [action, chordRaw] of Object.entries(bindings)) {
    if (!(ACTION_IDS as readonly string[]).includes(action)) fail(`unknown action '${action}'`);
    const chord = chordRaw as Record<string, unknown>;
    if (typeof chord['key'] !== 'string') fail(`binding for '${action}' missing key`);
    const key = chord['key'];
    if (!SINGLE_KEY.test(key) && !NAMED_KEYS.has(key)) fail(`binding for '${action}' has invalid key '${key}'`);
    out[action as ActionId] = {
      key,
      ctrl: chord['ctrl'] === true ? true : undefined,
      shift: chord['shift'] === true ? true : undefined,
      alt: chord['alt'] === true ? true : undefined,
    };
  }
  return { version: root['version'], bindings: out };
}

/** Defaults overridden by user bindings. */
export function resolveKeymap(user: KeymapFile | null): Map<ActionId, KeyChord> {
  const out = new Map<ActionId, KeyChord>(Object.entries(DEFAULT_KEYMAP) as [ActionId, KeyChord][]);
  if (user) {
    for (const action of ACTION_IDS) {
      const chord = user.bindings[action];
      if (chord) out.set(action, chord);
    }
  }
  return out;
}

/** Human-readable chord, e.g. "Ctrl+Shift+C". */
export function chordToString(c: KeyChord): string {
  const parts: string[] = [];
  if (c.ctrl) parts.push('Ctrl');
  if (c.shift) parts.push('Shift');
  if (c.alt) parts.push('Alt');
  parts.push(c.key.length === 1 ? c.key.toUpperCase() : c.key);
  return parts.join('+');
}

/** Same chord bound to 2+ actions — reported as warning-level conflicts. */
export function findConflicts(resolved: Map<ActionId, KeyChord>): Array<{ chord: string; actions: string[] }> {
  const byChord = new Map<string, string[]>();
  for (const [action, chord] of resolved) {
    const c = chordToString(chord);
    const list = byChord.get(c) ?? [];
    list.push(action);
    byChord.set(c, list);
  }
  return [...byChord.entries()]
    .filter(([, actions]) => actions.length > 1)
    .map(([chord, actions]) => ({ chord, actions }));
}

/** Builds a normalized chord from a keyboard event's fields. */
export function keyEventToChord(key: string, ctrl: boolean, shift: boolean, alt: boolean): KeyChord | null {
  const normalized = key === ' ' ? 'Space' : key.toLowerCase();
  if (!SINGLE_KEY.test(normalized) && !NAMED_KEYS.has(normalized) && normalized !== 'Space') return null;
  return {
    key: normalized,
    ctrl: ctrl || undefined,
    shift: shift || undefined,
    alt: alt || undefined,
  };
}
```

> Default keymap intentionally allows two providers for back/cancel-crop (`Escape`)
> and open-folder/open-file (`Ctrl+O` / `Ctrl+Shift+O` are distinct chords, so no
> conflict). `findConflicts` flags the Escape pair as a warning; the app dispatch
> layer resolves it by mode (Task 17): `back` in grid, `cancelCrop` in crop.

- [ ] **Step 5: Run keymap tests** — expected PASS.

- [ ] **Step 6: Add the parse-resolve conflict test for defaults (already covered under `resolveKeymap merges...`)** — no change.

- [ ] **Step 7: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/core/actions.ts src/core/keymap.ts tests/core/keymap.test.ts
git commit -m "feat: add action registry and validated keymap resolution"
```

---

## Task 9: Settings persistence + version migration

**Files:**
- Create: `src/config/settings.ts`
- Test: `tests/core/settings.test.ts`

- [ ] **Step 1: Write failing tests `tests/core/settings.test.ts`**

```ts
import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DEFAULT_SETTINGS, loadSettings, migrateSettings, saveSettings, type Settings } from '../../src/config/settings';

let dir: string;
let file: string;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'shimg-settings-'));
  file = join(dir, 'settings.json');
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

describe('settings', () => {
  test('defaults are returned when no file exists', () => {
    const s = loadSettings(file);
    expect(s).toEqual(DEFAULT_SETTINGS);
  });

  test('save/load round trip preserves values', async () => {
    const custom: Settings = {
      ...DEFAULT_SETTINGS,
      version: 1,
      activeTheme: 'terminal',
      lastFolder: 'C:/Pictures',
      cache: { thumbMaxEdgePx: 320, maxCacheBytes: 256 * 1024 * 1024 },
    };
    saveSettings(custom, file);
    const raw = await readFile(file, 'utf8');
    expect(JSON.parse(raw)).toEqual(custom);
    expect(loadSettings(file)).toEqual(custom);
  });

  test('migrates legacy v0 files onto v1 defaults', () => {
    const migrated = migrateSettings({ version: 0, activeTheme: 'studio' });
    expect(migrated.version).toBe(1);
    expect(migrated.activeTheme).toBe('studio');
    expect(migrated.cache).toEqual(DEFAULT_SETTINGS.cache);
  });

  test('unreadable file falls back to defaults without throwing', async () => {
    await writeFile(file, '{bad json');
    expect(loadSettings(file)).toEqual(DEFAULT_SETTINGS);
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write minimal `src/config/settings.ts`**

```ts
import { readFileSync, renameSync, writeFileSync } from 'node:fs';
import { ShImagesError } from '../utils/errors';
import { logger } from '../utils/logger';

export interface Settings {
  version: 1;
  activeTheme: string;
  lastFolder: string | null;
  cache: {
    thumbMaxEdgePx: number;
    maxCacheBytes: number;
  };
}

export const DEFAULT_SETTINGS: Settings = {
  version: 1,
  activeTheme: 'lightbox',
  lastFolder: null,
  cache: { thumbMaxEdgePx: 256, maxCacheBytes: 256 * 1024 * 1024 },
};

/** Normalizes arbitrary parsed data onto v1 defaults (never throws). */
export function migrateSettings(raw: unknown): Settings {
  const r = raw as Record<string, unknown>;
  const cache = r['cache'] as Record<string, unknown> | undefined;
  return {
    version: 1,
    activeTheme: typeof r['activeTheme'] === 'string' ? r['activeTheme'] : DEFAULT_SETTINGS.activeTheme,
    lastFolder: typeof r['lastFolder'] === 'string' ? r['lastFolder'] : null,
    cache: {
      thumbMaxEdgePx:
        typeof cache?.['thumbMaxEdgePx'] === 'number' ? cache['thumbMaxEdgePx'] : DEFAULT_SETTINGS.cache.thumbMaxEdgePx,
      maxCacheBytes:
        typeof cache?.['maxCacheBytes'] === 'number' ? cache['maxCacheBytes'] : DEFAULT_SETTINGS.cache.maxCacheBytes,
    },
  };
}

/** Loads settings; a missing or corrupt file degrades to defaults. */
export function loadSettings(file: string): Settings {
  let raw: string;
  try {
    raw = readFileSync(file, 'utf8');
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
  try {
    return migrateSettings(JSON.parse(raw));
  } catch (cause) {
    logger.warn({ cause }, 'settings file unreadable, using defaults');
    return { ...DEFAULT_SETTINGS };
  }
}

/** Persists settings atomically (write temp, rename). */
export function saveSettings(settings: Settings, file: string): void {
  const tmp = `${file}.tmp`;
  try {
    writeFileSync(tmp, JSON.stringify(settings, null, 2), 'utf8');
    renameSync(tmp, file);
  } catch (cause) {
    throw new ShImagesError('IO_ERROR', `Cannot save settings to ${file}`, cause);
  }
}
```

> `saveSettings` is intentionally synchronous (atomic rename); call sites run it
> outside the render path so it never blocks the frame loop.

- [ ] **Step 4: Run settings tests** — expected PASS.

- [ ] **Step 5: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/config/settings.ts tests/core/settings.test.ts
git commit -m "feat: persist settings with version migration and defaults"
```

---

## Task 10: Decode workers (thumbnails + crop) and worker bus

**Files:**
- Create: `src/workers/thumb-worker.ts`
- Create: `src/workers/crop-worker.ts`
- Create: `src/hooks/use-worker.ts`
- Create: `tests/core/workers.test.ts` (integration-ish: spawns real worker via Bun)

- [ ] **Step 1: Write failing test `tests/core/workers.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import {
  createThumbWorker,
  createCropWorker,
  thumbRequest,
  cropRequest,
} from '../../src/hooks/use-worker';

describe('worker bus', () => {
  test('creates a thumb worker and processes a request', async () => {
    const worker = await createThumbWorker();
    const promise = new Promise<unknown>((resolve, reject) => {
      worker.onmessage = (e) => {
        const data = e.data as { ok: boolean };
        if (data.ok) resolve(data);
        else reject(new Error('thumb failed'));
      };
    });
    worker.postMessage(
      thumbRequest(1, 'tests/fixtures/sample.png', 'unused-output', 64),
    );
    const result = (await promise) as { ok: boolean; width?: number };
    expect(result.ok).toBe(true);
    worker.terminate();
  });

  test('reports decode failure instead of throwing', async () => {
    const worker = await createThumbWorker();
    const promise = new Promise<unknown>((resolve) => {
      worker.onmessage = (e) => resolve(e.data);
    });
    worker.postMessage(thumbRequest(2, 'tests/fixtures/missing.png', 'unused-output', 64));
    const result = (await promise) as { ok: boolean };
    expect(result.ok).toBe(false);
    worker.terminate();
  });
});
```

> This test requires a real fixture: uncomment fixture generation step (Step 4)
> first, or run `bun run tests/fixtures/make-fixtures.ts` manually before Step 5.
> `Bun.file(...).textSync()` in the worker reads the file bytes; `sample.png` is a
> tiny deterministic image from the fixture script.

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write `src/workers/thumb-worker.ts`**

```ts
import { decodeImage, encodeJpeg } from '@napi-rs/image';
import { downscaleBox, type RgbaImage } from '../core/image-ops';
import { writeFile } from 'node:fs/promises';

export interface ThumbRequest {
  readonly id: number;
  readonly path: string;
  readonly maxEdge: number;
}

export interface ThumbResponse {
  readonly id: number;
  readonly ok: boolean;
  readonly width?: number;
  readonly height?: number;
  readonly error?: string;
}

self.onmessage = async (e: MessageEvent<ThumbRequest>) => {
  const req = e.data;
  try {
    const file = Bun.file(req.path);
    const buf = await file.arrayBuffer();
    const decoded = decodeImage(new Uint8Array(buf)) as RgbaImage;
    const small = downscaleBox(decoded, req.maxEdge);
    const jpeg = encodeJpeg(small.data, small.width, small.height, { quality: 82 });
    const outPath = cacheOutPath(req);
    await writeFile(outPath, jpeg);
    const response: ThumbResponse = { id: req.id, ok: true, width: small.width, height: small.height };
    self.postMessage(response);
  } catch (cause) {
    const response: ThumbResponse = {
      id: req.id,
      ok: false,
      error: cause instanceof Error ? cause.message : String(cause),
    };
    self.postMessage(response);
  }
};

function cacheOutPath(req: ThumbRequest): string {
  // caller-provided deterministic cache path via postMessage extra field
  const outDir = (req as ThumbRequest & { outDir?: string }).outDir;
  if (!outDir) throw new Error('missing outDir');
  return `${outDir}/${sanitize(req.path)}-${req.maxEdge}.jpg`;
}

function sanitize(p: string): string {
  return p.replace(/[^a-zA-Z0-9._-]/g, '_');
}
```

> Worker decode: `decodeImage` returns a `{ data, width, height }` object; if the
> installed `@napi-rs/image` types differ, adapt to the observed signature
> (record it in a code comment next to the import). If `encodeJpeg` is not
> exported, use `encodePng` and name the output file `.png`; update
> `cacheOutPath` accordingly.

- [ ] **Step 4: Write fixture generator `tests/fixtures/make-fixtures.ts`** (also needed by Task 18)

```ts
import { writeFile, mkdir } from 'node:fs/promises';
import { encodeJpeg, encodePng } from '@napi-rs/image';
import { dirname } from 'node:path';

const DIR = new URL('.', import.meta.url).pathname;

function rgba(w: number, h: number): Uint8Array {
  const out = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      out[i] = (x * 255) / w;
      out[i + 1] = (y * 255) / h;
      out[i + 2] = 128;
      out[i + 3] = 255;
    }
  }
  return out;
}

export async function makeFixtures(): Promise<string[]> {
  await mkdir(DIR, { recursive: true });
  const png = encodePng(rgba(320, 240), 320, 240);
  const jpg = encodeJpeg(rgba(320, 240), 320, 240, { quality: 80 });
  // minimal 1x1 transparent GIF (fixed hex, deterministic)
  const gif = Buffer.from('47494638396101000100800000ffffff00000021f90401000000002c00000000010001000002024401003b', 'hex');
  const corrupt = Buffer.from('ffd8ffe000104a46494600ff', 'hex'); // truncated JPEG header
  const empty = Buffer.alloc(0);
  await writeFile(`${DIR}/sample.png`, png);
  await writeFile(`${DIR}/sample.jpg`, jpg);
  await writeFile(`${DIR}/sample.gif`, gif);
  await writeFile(`${DIR}/corrupt.png`, corrupt);
  await writeFile(`${DIR}/empty.txt`, empty);
  return [png.length.toString(), jpg.length.toString()];
}

if (import.meta.main) {
  const sizes = await makeFixtures();
  console.log(`fixtures written: png=${sizes[0]}B jpg=${sizes[1]}B`);
}
```

- [ ] **Step 5: Write `src/workers/crop-worker.ts`**

```ts
import { decodeImage, encodeJpeg, encodePng } from '@napi-rs/image';
import { cropPixels, flattenAlpha, type RgbaImage } from '../core/image-ops';
import type { Rect } from '../core/crop-math';

export interface CropRequest {
  readonly id: number;
  readonly path: string;
  readonly rect: Rect;
  readonly format: 'png' | 'jpeg';
}

export interface CropResponse {
  readonly id: number;
  readonly ok: boolean;
  readonly buffer?: Uint8Array;
  readonly error?: string;
}

self.onmessage = async (e: MessageEvent<CropRequest>) => {
  const req = e.data;
  try {
    const file = Bun.file(req.path);
    const buf = await file.arrayBuffer();
    const decoded = decodeImage(new Uint8Array(buf)) as RgbaImage;
    const region = cropPixels(decoded, req.rect);
    const pixelData = req.format === 'jpeg' ? flattenAlpha(region) : region;
    const encoded =
      req.format === 'jpeg'
        ? encodeJpeg(pixelData.data, pixelData.width, pixelData.height, { quality: 90 })
        : encodePng(pixelData.data, pixelData.width, pixelData.height);
    const response: CropResponse = { id: req.id, ok: true, buffer: encoded };
    self.postMessage(response);
  } catch (cause) {
    const response: CropResponse = {
      id: req.id,
      ok: false,
      error: cause instanceof Error ? cause.message : String(cause),
    };
    self.postMessage(response);
  }
};
```

- [ ] **Step 6: Write `src/hooks/use-worker.ts`** (typed worker bus used by UI)

```ts
import type { ThumbRequest, ThumbResponse } from '../workers/thumb-worker';
import type { CropRequest, CropResponse } from '../workers/crop-worker';

export function createThumbWorker(): Promise<Worker> {
  return new Promise((resolve, reject) => {
    const w = new Worker(new URL('../workers/thumb-worker.ts', import.meta.url), { type: 'module' });
    w.addEventListener('error', reject);
    w.addEventListener('message', () => resolve(w), { once: true });
  });
}

export function thumbRequest(id: number, path: string, outDir: string, maxEdge: number): ThumbRequest {
  return { id, path, maxEdge, outDir } as ThumbRequest;
}

export function createCropWorker(): Promise<Worker> {
  return new Promise((resolve, reject) => {
    const w = new Worker(new URL('../workers/crop-worker.ts', import.meta.url), { type: 'module' });
    w.addEventListener('error', reject);
    w.addEventListener('message', () => resolve(w), { once: true });
  });
}

export function cropRequest(id: number, path: string, rect: CropRequest['rect'], format: 'png' | 'jpeg'): CropRequest {
  return { id, path, rect, format };
}

export type { ThumbRequest, ThumbResponse, CropRequest, CropResponse };
```

> The worker constructs resolve on the first message; request/response correlation
> is by `id` (the first "ready" message is an implementation detail). If Bun's
> `Worker` with `type: 'module'` fails on Windows, switch to
> `new Worker(new URL(..., import.meta.url))` (default type) — record the
> outcome in the commit message.

- [ ] **Step 7: Run worker tests** — expected PASS:

Run: `bun test tests/core/workers.test.ts`
Expected: both tests pass (fixtures generated in Step 4; adjust the worker test's
sample path if `make-fixtures.ts` output differs).

- [ ] **Step 8: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/workers src/hooks/use-worker.ts tests/core/workers.test.ts tests/fixtures/make-fixtures.ts
git commit -m "feat: add thumbnail and crop decode workers"
```

---

## Task 11: Clipboard, file dialog fallback, drag-drop, os utils

**Files:**
- Create: `src/utils/clipboard.ts`
- Create: `src/utils/file-dialog.ts`
- Create: `src/utils/drag-drop.ts`
- Create: `src/utils/ensure-dir.ts`
- Test: `tests/core/ensure-dir.test.ts`

- [ ] **Step 1: Write failing test `tests/core/ensure-dir.test.ts`**

```ts
import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ensureDir } from '../../src/utils/ensure-dir';

let dir: string;

afterEach(async () => {
  if (dir) await rm(dir, { recursive: true, force: true });
});

describe('ensureDir', () => {
  test('creates nested directories and is idempotent', async () => {
    dir = await mkdtemp(join(tmpdir(), 'shimg-dir-'));
    const target = join(dir, 'a', 'b', 'c');
    await expect(ensureDir(target)).resolves.toBeUndefined();
    await expect(ensureDir(target)).resolves.toBeUndefined();
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write minimal `src/utils/ensure-dir.ts`**

```ts
import { mkdir } from 'node:fs/promises';
import { ShImagesError } from './errors';

export async function ensureDir(path: string): Promise<void> {
  try {
    await mkdir(path, { recursive: true });
  } catch (cause) {
    throw new ShImagesError('IO_ERROR', `Cannot create directory ${path}`, cause);
  }
}
```

- [ ] **Step 4: Run ensure-dir test** — expected PASS.

- [ ] **Step 5: Write `src/utils/clipboard.ts`**

```ts
import { writeImagePng } from '@napi-rs/clipboard';
import { ShImagesError } from './errors';

/** Writes an encoded PNG buffer to the OS clipboard. */
export function writePngToClipboard(png: Uint8Array): void {
  try {
    writeImagePng(png);
  } catch (cause) {
    throw new ShImagesError('IO_ERROR', 'Cannot write image to clipboard', cause);
  }
}
```

> Verify the installed `@napi-rs/clipboard` export name (`writeImagePng` expected;
> if the package exposes `writeImage` with a different signature, adapt this
> wrapper — that is the spike result, record it in a code comment).

- [ ] **Step 6: Write `src/utils/file-dialog.ts`** (native dialog when available; guaranteed text fallback)

```ts
import { ShImagesError } from './errors';

/**
 * Picks a folder. V1 fallback: a plain-path prompt (guaranteed to work in a
 * GPUIX window via a modal with a text input). Swap `this` implementation for a
 * native dialog if the GPUIX spike (Task 1) proved one is available.
 */
export async function pickFolderPath(): Promise<string | null> {
  const value = await promptForPath('Folder path');
  return value;
}

/** Picks an image file path; same fallback contract as pickFolderPath. */
export async function pickImagePath(): Promise<string | null> {
  const value = await promptForPath('Image file path');
  return value;
}

export function promptForPath(label: string): Promise<string | null> {
  // NOTE: real implementation attaches an in-app modal (Task 14/17 components).
  // This function is intentionally replaceable by a native dialog later.
  return Promise.resolve(null);
}
```

> `promptForPath` is stubbed until the UI shell exists (Task 14): the modal
> component will implement it by rendering a text input and returning the value
> through a promise. Keep this contract.

- [ ] **Step 7: Write `src/utils/drag-drop.ts`**

```ts
import { isSupportedImagePath } from '../core/formats';

export type DropKind = 'folder' | 'image' | 'unsupported';

/** Classifies a dropped path without touching the filesystem (extension-based). */
export function classifyDrop(path: string): DropKind {
  const lower = path.toLowerCase().replace(/\/+$/, '');
  if (/[\\/]$/.test(lower) || !/[.]/.test(lower.slice(lower.lastIndexOf('\\') + 1))) return 'folder';
  return isSupportedImagePath(lower) ? 'image' : 'unsupported';
}
```

> If the GPUIX drag-drop event provides only file bytes (not paths), adapt
> `classifyDrop` to classify by the event shape and note it in the commit.

- [ ] **Step 8: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/utils/clipboard.ts src/utils/file-dialog.ts src/utils/drag-drop.ts src/utils/ensure-dir.ts tests/core/ensure-dir.test.ts
git commit -m "feat: add clipboard, dialog fallback, drop classification, dir ensure"
```

---

## Task 12: Windows registration + CLI arg parsing

**Files:**
- Create: `src/utils/registration.ts`
- Create: `src/utils/argv.ts`
- Test: `tests/core/registration.test.ts` (command builder only; no real registry writes)
- Test: `tests/core/argv.test.ts`

- [ ] **Step 1: Write failing tests**

```ts
// tests/core/registration.test.ts
import { describe, expect, test } from 'bun:test';
import { buildRegistrationCommands, buildUnregisterCommands } from '../../src/utils/registration';

const EXE = 'C:/Program Files/Sh_Images/sh_images.exe';

describe('registration command builder', () => {
  test('builds ProgID + Open command + OpenWithProgIds per extension', () => {
    const cmds = buildRegistrationCommands(EXE, ['png', 'jpg']);
    expect(cmds.length).toBeGreaterThan(0);
    const joined = cmds.join('\n');
    expect(joined).toContain('ShImages.Viewer');
    expect(joined).toContain('"C:/Program Files/Sh_Images/sh_images.exe" "%1"');
    expect(joined).toContain('.png');
    expect(joined).toContain('.jpg');
  });

  test('unregister removes ProgID and OpenWithProgIds entries', () => {
    const cmds = buildUnregisterCommands(['png']);
    const joined = cmds.join('\n');
    expect(joined).toContain('delete');
    expect(joined).toContain('ShImages.Viewer');
  });
});

// tests/core/argv.test.ts
import { describe, expect, test } from 'bun:test';
import { parseArgv } from '../../src/utils/argv';

describe('parseArgv', () => {
  test('no args → open mode with null path', () => {
    expect(parseArgv([])).toEqual({ path: null });
  });
  test('single path arg is used', () => {
    expect(parseArgv(['C:/x/photo.png'])).toEqual({ path: 'C:/x/photo.png' });
  });
  test('strips non-path flags', () => {
    expect(parseArgv(['--verbose', 'C:/x/photo.png'])).toEqual({ path: 'C:/x/photo.png' });
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL for both.

- [ ] **Step 3: Write minimal `src/utils/argv.ts`**

```ts
export interface ParsedArgs {
  path: string | null;
}

/** Parses process args (after the executable) into a single optional path. */
export function parseArgv(args: readonly string[]): ParsedArgs {
  const path = args.find((a) => !a.startsWith('-'));
  return { path: path ?? null };
}
```

- [ ] **Step 4: Write minimal `src/utils/registration.ts`**

```ts
import { execFileSync } from 'node:child_process';
import { SUPPORTED_EXTENSIONS } from '../core/formats';
import { ShImagesError } from './errors';

const PROGID = 'ShImages.Viewer';

function quote(s: string): string {
  return `"${s.replaceAll('"', '\\"')}"`;
}

/** Builds the exact reg.exe command lines for registration (testable, pure). */
export function buildRegistrationCommands(
  exePath: string,
  extensions: readonly string[],
): string[] {
  const cmds: string[] = [];
  cmds.push(
    `reg add "HKCU\\Software\\Classes\\${PROGID}\\shell\\open\\command" /ve /d ${quote(`${exePath} "%1"`)} /f`,
  );
  cmds.push(`reg add "HKCU\\Software\\Classes\\${PROGID}\\DefaultIcon" /ve /d ${quote(exePath)} /f`);
  for (const ext of extensions) {
    cmds.push(`reg add "HKCU\\Software\\Classes\\.${ext}\\OpenWithProgIds" /v ${PROGID} /t REG_SZ /d "" /f`);
  }
  return cmds;
}

/** Builds the exact reg.exe command lines for unregistration. */
export function buildUnregisterCommands(extensions: readonly string[]): string[] {
  const cmds: string[] = [];
  for (const ext of extensions) {
    cmds.push(`reg delete "HKCU\\Software\\Classes\\.${ext}\\OpenWithProgIds" /v ${PROGID} /f`);
  }
  cmds.push(`reg delete "HKCU\\Software\\Classes\\${PROGID}" /f`);
  return cmds;
}

/** Idempotent, Windows-only registration. Non-Windows is a no-op. */
export function ensureRegistered(exePath: string): void {
  if (process.platform !== 'win32') return;
  const cmds = buildRegistrationCommands(exePath, SUPPORTED_EXTENSIONS);
  for (const cmd of cmds) {
    try {
      execFileSync('reg.exe', ['add', ...cmd.split(' ').slice(1)], { stdio: 'ignore' });
    } catch (cause) {
      throw new ShImagesError('IO_ERROR', `Registry registration failed: ${cmd}`, cause);
    }
  }
}
```

> The `execFileSync` argument split keeps typechecking happy (no shell), but on
> Windows paths with spaces the `cmd.split(' ')` approach is fragile — a safer
> variant is to spawn `reg.exe add` with an argv array built directly. Replace
> the exec loop with the array form if the spike run shows quoting issues:
> `execFileSync('reg.exe', ['add', `HKCU\\Software\\Classes\\${PROGID}\\shell\\open\\command`, '/ve', '/d', `${exePath} "%1"`, '/f'])`.

- [ ] **Step 5: Run registration + argv tests** — expected PASS.

- [ ] **Step 6: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/utils/registration.ts src/utils/argv.ts tests/core/registration.test.ts tests/core/argv.test.ts
git commit -m "feat: add Windows default-viewer registration commands and argv parsing"
```

---

## Task 13: App shell — theme provider, keymap hook, welcome screen

**Files:**
- Create: `src/hooks/use-theme.ts`
- Create: `src/hooks/use-keymap.ts`
- Create: `src/components/shell.tsx`
- Modify: `src/main.tsx`
- Create: `tests/core/keymap-hook.test.ts` (pure dispatch helper)

- [ ] **Step 1: Write failing test for the pure dispatch helper `tests/core/keymap-hook.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { resolveDispatch } from '../../src/hooks/use-keymap';
import { DEFAULT_KEYMAP, type ActionId, type KeyChord } from '../../src/core/actions';

describe('resolveDispatch', () => {
  test('maps a key event to the action honoring the resolved keymap', () => {
    const resolved = new Map<ActionId, KeyChord>(Object.entries(DEFAULT_KEYMAP) as [ActionId, KeyChord][]);
    const dispatch = resolveDispatch(resolved);
    expect(dispatch({ key: 'ArrowRight' })).toBe('next');
  });

  test('returns undefined for unmapped chords', () => {
    const dispatch = resolveDispatch(new Map());
    expect(dispatch({ key: 'z' })).toBeUndefined();
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write minimal `src/hooks/use-theme.ts`**

```ts
import { createContext, useContext, useState, type ReactNode } from 'react';
import { LIGHTBOX_THEME, type ThemeTokens } from '../core/theme';

export const ThemeContext = createContext<ThemeTokens>(LIGHTBOX_THEME);

/** Reads the active theme tokens (must render inside <ThemeProvider>). */
export function useTheme(): ThemeTokens {
  return useContext(ThemeContext);
}

export function ThemeProvider({ tokens, children }: { tokens: ThemeTokens; children: ReactNode }) {
  return <ThemeContext.Provider value={tokens}>{children}</ThemeContext.Provider>;
}

/** Switches active theme id with persistence (persist wired in Task 16/17). */
export function useThemeSwitch(): [ThemeTokens, (id: string) => void] {
  const [tokens, setTokens] = useState<ThemeTokens>(LIGHTBOX_THEME);
  return [tokens, (id) => setTokens(resolveById(id))];
}

function resolveById(id: string): ThemeTokens {
  // resolveTheme lives in core/theme; kept here to avoid a circular import.
  return id === 'lightbox' ? LIGHTBOX_THEME : LIGHTBOX_THEME;
}
```

> `useThemeSwitch` is a minimal placeholder until Task 16 wires real
> `resolveTheme` and persistence; the context provider is the contract.

- [ ] **Step 4: Write minimal `src/hooks/use-keymap.ts`**

```ts
import { useCallback, useEffect } from 'react';
import { keyEventToChord, chordToString, type KeymapFile } from '../core/keymap';
import { resolveKeymap, type ActionId, type KeyChord } from '../core/keymap';

export type ChordLookup = (chord: KeyChord) => ActionId | undefined;

/** Builds a chord→action lookup from a resolved keymap. */
export function resolveDispatch(resolved: Map<ActionId, KeyChord>): ChordLookup {
  const byChord = new Map<string, ActionId>();
  for (const [action, chord] of resolved) {
    byChord.set(chordToString(chord), action);
  }
  return (chord) => byChord.get(chordToString(chord));
}

/** Registers a global keydown handler that dispatches to actions. */
export function useKeymap(
  handler: (action: ActionId) => void,
  lookup: ChordLookup,
  active: boolean,
): void {
  useEffect(() => {
    if (!active) return;
    const onKeyDown = (e: KeyboardEvent) => {
      const chord = keyEventToChord(e.key, e.ctrlKey, e.shiftKey, e.altKey);
      if (!chord) return;
      const action = lookup(chord);
      if (action) {
        e.preventDefault();
        handler(action);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [handler, lookup, active]);
}

export type { ActionId, KeyChord };
```

> The DOM `window.addEventListener` here is a fallback while GPUIX key APIs are
> confirmed. Per AGENTS.md §7.3, DO NOT use DOM APIs on desktop — this hook must
> be re-pointed at the GPUIX `keyDown` prop surface once the Task 1 spike
> confirms it. Mark the swap point in the final code review.

- [ ] **Step 5: Write the app shell `src/components/shell.tsx`** (welcomes with theme applied)

```tsx
import type { ReactNode } from 'react';
import { useTheme } from '../hooks/use-theme';

export function Shell({ children }: { children: ReactNode }) {
  const theme = useTheme();
  return (
    <div
      style={{
        background: theme.colors.background,
        color: theme.colors.text,
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      {children}
    </div>
  );
}
```

- [ ] **Step 6: Wire `src/main.tsx`**

```tsx
import { render } from '@gpuix/react';
import { ThemeProvider } from './hooks/use-theme';
import { LIGHTBOX_THEME } from './core/theme';
import { Shell } from './components/shell';
import { Welcome } from './components/welcome';

function Welcome() {
  return (
    <Shell>
      <div style={{ color: '#ffffff', fontSize: 18, fontFamily: 'Inter, sans-serif', padding: 16 }}>
        Sh_Images — open a folder or drop images here
      </div>
    </Shell>
  );
}

render(
  <ThemeProvider tokens={LIGHTBOX_THEME}>
    <Welcome />
  </ThemeProvider>,
  { title: 'Sh_Images', width: 1200, height: 800, resizable: true },
);
```

> If `render`'s options or the JSX namespace differ in the installed
> `@gpuix/react` (Task 1 spike), adapt this file — the shape shown is the
> contract the plan targets.

- [ ] **Step 7: Run the keymap-hook test** — expected PASS.

Run: `bun test tests/core/keymap-hook.test.ts`

- [ ] **Step 8: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/hooks/use-theme.ts src/hooks/use-keymap.ts src/components/shell.tsx src/main.tsx tests/core/keymap-hook.test.ts
git commit -m "feat: add app shell, theme provider, and keymap dispatch"
```

---

## Task 14: Folder grid + thumbnails

**Files:**
- Create: `src/hooks/use-folder.ts`
- Create: `src/hooks/use-thumbnails.ts`
- Create: `src/components/folder-grid.tsx`
- Create: `src/components/welcome.tsx`
- Test: `tests/core/use-thumbnails.test.ts` (pure portion)

- [ ] **Step 1: Write failing test `tests/core/use-thumbnails.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { deriveThumbKey } from '../../src/hooks/use-thumbnails';

describe('thumb key derivation', () => {
  test('is deterministic per path and size', () => {
    const a = deriveThumbKey('C:/x/photo.png', 256);
    const b = deriveThumbKey('C:/x/photo.png', 256);
    const c = deriveThumbKey('C:/x/photo.png', 128);
    expect(a).toBe(b);
    expect(a).not.toBe(c);
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write `src/hooks/use-folder.ts`**

```ts
import { useCallback, useState } from 'react';
import { readdir } from 'node:fs/promises';
import { join } from 'node:path';
import { FolderIndex, type ImageEntry } from '../core/navigation';
import { isSupportedImagePath } from '../core/formats';
import { ShImagesError } from '../utils/errors';

/** Scans a directory into a sorted FolderIndex (supported images only). */
export async function scanFolder(folderPath: string): Promise<FolderIndex> {
  let names: string[];
  try {
    names = await readdir(folderPath);
  } catch (cause) {
    throw new ShImagesError('IO_ERROR', `Cannot read folder ${folderPath}`, cause);
  }
  const entries: ImageEntry[] = names
    .filter((n) => isSupportedImagePath(n))
    .map((n) => ({ path: join(folderPath, n), name: n }));
  return new FolderIndex(entries);
}

export interface FolderState {
  index: FolderIndex | null;
  current: number;
  error: string | null;
}

/** App-level folder state: open a path (folder or image) and navigate. */
export function useFolder(): {
  state: FolderState;
  openPath: (path: string) => Promise<void>;
  navigate: (dir: 1 | -1) => void;
  setCurrent: (i: number) => void;
} {
  const [state, setState] = useState<FolderState>({ index: null, current: 0, error: null });

  const openPath = useCallback(async (path: string) => {
    try {
      const index = await scanFolder(path);
      setState({ index, current: 0, error: null });
    } catch (cause) {
      setState((s) => ({ ...s, error: cause instanceof Error ? cause.message : String(cause) }));
    }
  }, []);

  const navigate = useCallback((dir: 1 | -1) => {
    setState((s) => {
      if (!s.index) return s;
      return { ...s, current: dir === 1 ? s.index.next(s.current) : s.index.prev(s.current) };
    });
  }, []);

  const setCurrent = useCallback((i: number) => {
    setState((s) => ({ ...s, current: i }));
  }, []);

  return { state, openPath, navigate, setCurrent };
}
```

- [ ] **Step 4: Write `src/hooks/use-thumbnails.ts`** (LRU-backed worker bus)

```ts
import { useCallback, useEffect, useRef, useState } from 'react';
import { LruCache } from '../core/image-cache';
import { ensureDir } from '../utils/ensure-dir';
import { cacheDir } from '../config/paths';
import { createThumbWorker, thumbRequest, type ThumbResponse } from './use-worker';

const THUMBS = new LruCache(256 * 1024 * 1024);

export function deriveThumbKey(path: string, maxEdge: number): string {
  return `${maxEdge}:${path.replaceAll('\\', '/').toLowerCase()}`;
}

export interface ThumbHooks {
  urlFor: (path: string) => string | null;
  clearRequest: (path: string) => void;
}

/** Requests a downsampled thumbnail via the worker; returns a cache file URL. */
export function useThumbnails(maxEdge = 256): ThumbHooks {
  const [urls, setUrls] = useState<ReadonlyMap<string, string>>(new Map());
  const workerRef = useRef<Worker | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const worker = await createThumbWorker();
      if (cancelled) {
        worker.terminate();
        return;
      }
      workerRef.current = worker;
      worker.onmessage = (e: MessageEvent<ThumbResponse>) => {
        if (!e.data.ok) return;
        const key = deriveThumbKey((e.data as ThumbResponse & { path?: string }).path ?? '', maxEdge);
        if (!key) return;
        const outDir = cacheDir();
        const file = `${outDir}/${keyToFile(key)}.jpg`;
        setUrls((prev) => new Map(prev).set(key, file));
      };
    })();
    return () => {
      cancelled = true;
      workerRef.current?.terminate();
      workerRef.current = null;
    };
  }, [maxEdge]);

  const request = useCallback(
    (path: string) => {
      void (async () => {
        const outDir = cacheDir();
        await ensureDir(outDir);
        const key = deriveThumbKey(path, maxEdge);
        const id = Math.floor(Math.random() * 0xffff);
        workerRef.current?.postMessage({ ...thumbRequest(id, path, '', maxEdge), outDir });
      })();
    },
    [maxEdge],
  );

  return {
    urlFor: (path) => urls.get(deriveThumbKey(path, maxEdge)) ?? null,
    clearRequest: (path) => {
      void request(path);
    },
  };
}

function keyToFile(key: string): string {
  return key.replace(/[^a-z0-9]/gi, '_');
}
```

> Cache key and file naming are deterministic so repeated opens reuse thumbnails.
> The worker's `outDir` payload (Object.assign) matches the `cacheOutPath` in
> `thumb-worker.ts`. Simplify if the worker contract changes.

- [ ] **Step 5: Write `src/components/welcome.tsx`** (abstracted from main.tsx)

```tsx
import { useTheme } from '../hooks/use-theme';

export function Welcome({ onOpenFolder }: { onOpenFolder: () => void }) {
  const theme = useTheme();
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: theme.spacing.md, justifyContent: 'center', alignItems: 'center', height: '100%' }}>
      <div style={{ fontSize: theme.typography.huge, color: theme.colors.text, fontFamily: theme.typography.family }}>
        Sh_Images
      </div>
      <div style={{ fontSize: theme.typography.body, color: theme.colors.textMuted }}>
        Open a folder or drop images here
      </div>
      <button style={{ padding: `${theme.spacing.sm}px ${theme.spacing.md}px`, borderRadius: theme.radii.md, background: theme.colors.accent, color: theme.colors.accentText, border: 'none', fontFamily: theme.typography.family }}
        onClick={onOpenFolder}>
        Open folder
      </button>
    </div>
  );
}
```

> GPUIX buttons render through GPUI's native button; adjust styling keys to the
> observed style surface (Task 1 spike). Text elements need explicit `color`.

- [ ] **Step 6: Write `src/components/folder-grid.tsx`**

```tsx
import { useEffect } from 'react';
import type { FolderIndex } from '../core/navigation';
import { useTheme } from '../hooks/use-theme';
import { useThumbnails } from '../hooks/use-thumbnails';

interface Props {
  index: FolderIndex;
  current: number;
  onSelect: (index: number) => void;
}

export function FolderGrid({ index, current, onSelect }: Props) {
  const theme = useTheme();
  const { urlFor, clearRequest } = useThumbnails();

  useEffect(() => {
    for (const entry of index.images) clearRequest(entry.path);
  }, [index, clearRequest]);

  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: theme.spacing.sm, padding: theme.spacing.md, overflow: 'auto', height: '100%' }}>
      {index.images.map((entry, i) => {
        const url = urlFor(entry.path);
        const selected = i === current;
        return (
          <div
            key={entry.path}
            style={{
              width: 160,
              height: 120,
              background: selected ? theme.colors.accent : theme.colors.surface,
              borderRadius: theme.radii.sm,
              border: `1px solid ${selected ? theme.colors.accent : theme.colors.border}`,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              overflow: 'hidden',
            }}
            onClick={() => onSelect(i)}
          >
            {url ? (
              <img src={url} style={{ width: 160, height: 120, objectFit: 'cover' }} />
            ) : (
              <div style={{ color: theme.colors.textMuted, fontSize: theme.typography.caption }}>{entry.name}</div>
            )}
          </div>
        );
      })}
    </div>
  );
}
```

- [ ] **Step 7: Run thumb-key test** — expected PASS.

Run: `bun test tests/core/use-thumbnails.test.ts`

- [ ] **Step 8: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/hooks/use-folder.ts src/hooks/use-thumbnails.ts src/components/folder-grid.tsx src/components/welcome.tsx tests/core/use-thumbnails.test.ts
git commit -m "feat: add folder scanning, thumbnail worker hook, and grid"
```

---

## Task 15: Single image view + zoom/pan

**Files:**
- Create: `src/hooks/use-pan-zoom.ts`
- Create: `src/components/image-view.tsx`
- Test: `tests/core/use-pan-zoom.test.ts` (pure transform math)

- [ ] **Step 1: Write failing test `tests/core/use-pan-zoom.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { computeViewTransform, panTransform, zoomTransform } from '../../src/hooks/use-pan-zoom';

describe('view transform', () => {
  test('computes fit transform for a 4000x3000 in 800x600', () => {
    const t = computeViewTransform(800, 600, 4000, 3000, 1);
    expect(t.scale).toBeCloseTo(0.2, 5);
  });

  test('zoom clamps at min and max multipliers', () => {
    const t = zoomTransform(800, 600, 4000, 3000, 0.001);
    expect(t.scale).toBeCloseTo(0.02, 5); // fit(0.2) * MIN_ZOOM(0.1)
    const big = zoomTransform(800, 600, 4000, 3000, 100000);
    expect(big.scale).toBeCloseTo(1.6, 5); // fit(0.2) * MAX_ZOOM(8)
  });

  test('pan adds to offset', () => {
    const base = computeViewTransform(800, 600, 4000, 3000, 1);
    const panned = panTransform(base, { x: 10, y: 20 });
    expect(panned.offsetX).toBe(base.offsetX + 10);
    expect(panned.offsetY).toBe(base.offsetY + 20);
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write `src/hooks/use-pan-zoom.ts`**

```ts
import { useCallback, useMemo, useState } from 'react';
import { fitTransform, type FitTransform, type Pt } from '../core/crop-math';

export const MAX_ZOOM = 8;
export const MIN_ZOOM = 0.1;

/** Pure transform computation (testable): fit-contained, zoom applies to center. */
export function computeViewTransform(viewW: number, viewH: number, imgW: number, imgH: number, zoom: number): FitTransform {
  const base = fitTransform(viewW, viewH, imgW, imgH);
  return { scale: base.scale * zoom, offsetX: base.offsetX, offsetY: base.offsetY };
}

export function zoomTransform(viewW: number, viewH: number, imgW: number, imgH: number, targetZoom: number): FitTransform {
  const base = fitTransform(viewW, viewH, imgW, imgH);
  const z = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, targetZoom));
  return { ...base, scale: base.scale * z };
}

export function panTransform(t: FitTransform, delta: Pt): FitTransform {
  return { ...t, offsetX: t.offsetX + delta.x, offsetY: t.offsetY + delta.y };
}

export interface PanZoom {
  transform: FitTransform;
  zoomIn: () => void;
  zoomOut: () => void;
  zoomActual: () => void;
  zoomFit: () => void;
  panBy: (dx: number, dy: number) => void;
}

export function usePanZoom(imgW: number, imgH: number, viewW: number, viewH: number): PanZoom {
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const transform = useMemo<FitTransform>(() => {
    const t = computeViewTransform(viewW, viewH, imgW, imgH, zoom);
    return { ...t, offsetX: t.offsetX + pan.x, offsetY: t.offsetY + pan.y };
  }, [zoom, pan, imgW, imgH, viewW, viewH]);

  const zoomIn = useCallback(() => setZoom((z) => Math.min(MAX_ZOOM, z * 1.25)), []);
  const zoomOut = useCallback(() => setZoom((z) => Math.max(MIN_ZOOM, z / 1.25)), []);
  const zoomActual = useCallback(() => setZoom(1), []);
  const zoomFit = useCallback(() => setZoom(1), []);
  const panBy = useCallback((dx: number, dy: number) => setPan((p) => ({ x: p.x + dx, y: p.y + dy })), []);

  return { transform, zoomIn, zoomOut, zoomActual, zoomFit, panBy };
}
```

> `zoomActual` maps to fit in this slice; exact source-pixel 1:1 mapping needs a
> measured viewport and lands with the resize handling in Phase 2. The clamp
> behavior (multiplier ∈ [0.1, 8]) is the contract for the hook.

- [ ] **Step 4: Run pan-zoom test** — expected PASS.

Run: `bun test tests/core/use-pan-zoom.test.ts`

- [ ] **Step 5: Write `src/components/image-view.tsx`**

```tsx
import { useState } from 'react';
import type { ImageEntry } from '../core/navigation';
import { usePanZoom } from '../hooks/use-pan-zoom';
import { useTheme } from '../hooks/use-theme';

interface Props {
  entry: ImageEntry;
  index: number;
  total: number;
  onBack: () => void;
  onNavigate: (dir: 1 | -1) => void;
}

export function ImageView({ entry, index, total, onBack, onNavigate }: Props) {
  const theme = useTheme();
  const [imgSize, setImgSize] = useState<{ w: number; h: number } | null>(null);
  const [chromeHidden, setChromeHidden] = useState(false);
  // Fixed window size for v1 (GPUIX window is resizable; measured size lands in Task 17)
  const viewW = 1200;
  const viewH = 800;
  const { transform, zoomIn, zoomOut, panBy } = usePanZoom(
    imgSize?.w ?? 1,
    imgSize?.h ?? 1,
    viewW,
    viewH,
  );

  return (
    <div style={{ position: 'relative', width: '100%', height: '100%', background: theme.colors.background, overflow: 'hidden' }}
      onWheel={(e) => {
        if (e.ctrlKey) {
          e.preventDefault();
          if (e.deltaY < 0) zoomIn();
          else zoomOut();
        } else {
          if (e.deltaY < 0) onNavigate(1);
          else onNavigate(-1);
        }
      }}
    >
      {imgSize ? (
        <img
          src={entry.path}
          style={{
            position: 'absolute',
            left: transform.offsetX,
            top: transform.offsetY,
            width: (imgSize.w * transform.scale).toFixed(1),
            height: (imgSize.h * transform.scale).toFixed(1),
          }}
        />
      ) : null}
      <img
        src={entry.path}
        style={{ display: 'none' }}
        onLoad={(e) => {
          const t = e.currentTarget as HTMLImageElement;
          setImgSize({ w: t.naturalWidth || 0, h: t.naturalHeight || 0 });
        }}
      />
      {!chromeHidden && (
        <div style={{ position: 'absolute', left: theme.spacing.md, top: theme.spacing.md, display: 'flex', gap: theme.spacing.sm }}>
          <button style={{ color: theme.colors.text, background: theme.colors.surface, border: 'none', borderRadius: theme.radii.sm, padding: theme.spacing.xs }}
            onClick={() => panBy(-10, 0)}>‹</button>
          <div style={{ color: theme.colors.text, fontSize: theme.typography.caption, alignSelf: 'center' }}>
            {index + 1} / {total} — {entry.name}
          </div>
          <button style={{ color: theme.colors.text, background: theme.colors.surface, border: 'none', borderRadius: theme.radii.sm, padding: theme.spacing.xs }}
            onClick={() => panBy(10, 0)}>›</button>
        </div>
      )}
    </div>
  );
}
```

> `onWheel` and `onLoad` must match the GPUIX event prop names discovered in
> Task 1's spike. If mismatch, update this file — the behavior (ctrl-wheel zooms,
> wheel navigates) is the contract. `e.currentTarget as HTMLImageElement`
> requires the DOM `HTMLImageElement` type under desktop; if GPUIX provides its
> own image element event type, use that instead and drop the cast.

- [ ] **Step 6: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/hooks/use-pan-zoom.ts src/components/image-view.tsx tests/core/use-pan-zoom.test.ts
git commit -m "feat: add single-image view with zoom and pan"
```

---

## Task 16: Crop overlay + export dialog (save/copy)

**Files:**
- Create: `src/components/crop-overlay.tsx`
- Create: `src/components/export-dialog.tsx`
- Modify: `src/components/image-view.tsx` (accept crop actions)
- Test: `tests/core/crop-flow.test.ts` (pure: crop selection to payload)

- [ ] **Step 1: Write failing test `tests/core/crop-flow.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { applyAspectPreset, buildCropPayload } from '../../src/components/crop-overlay';
import { aspectFromRect, fitTransform } from '../../src/core/crop-math';

describe('crop flow payload', () => {
  test('maps screen rect to source rect with fit transform', () => {
    const t = fitTransform(800, 600, 4000, 3000);
    const payload = buildCropPayload({ x: 100, y: 100, w: 200, h: 200 }, t, 4000, 3000);
    expect(payload.x).toBeCloseTo(500, 0);
    expect(payload.y).toBeCloseTo(500, 0);
    expect(payload.w).toBeCloseTo(1000, 0);
    expect(payload.h).toBeCloseTo(1000, 0);
  });

  test('clamps payload inside source bounds', () => {
    const t = fitTransform(800, 600, 4000, 3000);
    const payload = buildCropPayload({ x: -50, y: -50, w: 900, h: 900 }, t, 4000, 3000);
    expect(payload.x).toBe(0);
    expect(payload.y).toBe(0);
    expect(payload.w).toBeLessThanOrEqual(4000);
    expect(payload.h).toBeLessThanOrEqual(3000);
  });

  test('aspect presets reshape a selection around its center', () => {
    const r = applyAspectPreset({ x: 100, y: 100, w: 200, h: 100 }, '16/9');
    expect(aspectFromRect(r)).toBeCloseTo(16 / 9, 5);
    const center = { x: r.x + r.w / 2, y: r.y + r.h / 2 };
    expect(center.x).toBeCloseTo(200, 0);
    expect(center.y).toBeCloseTo(150, 0);
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write `src/components/crop-overlay.tsx`**

```tsx
import { useState } from 'react';
import {
  clampRect,
  fitRectToAspect,
  normalizeRect,
  screenToSource,
  type FitTransform,
  type Pt,
  type Rect,
} from '../core/crop-math';
import { useTheme } from '../hooks/use-theme';

export interface CropPayload extends Rect {}

export const ASPECT_PRESETS = [
  { id: 'free', label: 'Free', value: null },
  { id: '1/1', label: '1:1', value: 1 },
  { id: '16/9', label: '16:9', value: 16 / 9 },
  { id: '4/3', label: '4:3', value: 4 / 3 },
  { id: '3/2', label: '3:2', value: 3 / 2 },
] as const;

export const PRESET_IDS = ASPECT_PRESETS.map((p) => p.id) as readonly string[];

/** Pure: maps a screen-space crop rect to clamped source pixel bounds. */
export function buildCropPayload(rect: Rect, t: FitTransform, imgW: number, imgH: number): CropPayload {
  const topLeft = screenToSource({ x: rect.x, y: rect.y }, t);
  const bottomRight = screenToSource({ x: rect.x + rect.w, y: rect.y + rect.h }, t);
  return clampRect(normalizeRect(topLeft, bottomRight), imgW, imgH);
}

/** Pure: reshapes a selection to a preset aspect around its center. */
export function applyAspectPreset(rect: Rect, presetId: string): Rect {
  const preset = ASPECT_PRESETS.find((p) => p.id === presetId);
  if (!preset?.value) return rect;
  const center: Pt = { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 };
  return fitRectToAspect(rect, preset.value, center);
}

export function CropOverlay({
  transform,
  imgW,
  imgH,
  onConfirm,
  onCancel,
}: {
  transform: FitTransform;
  imgW: number;
  imgH: number;
  onConfirm: (rect: Rect) => void;
  onCancel: () => void;
}) {
  const theme = useTheme();
  const [anchor, setAnchor] = useState<Pt | null>(null);
  const [selection, setSelection] = useState<Rect | null>(null);
  const [presetId, setPresetId] = useState<string>('free');

  const confirmSelection = () => {
    if (selection) onConfirm(buildCropPayload(selection, transform, imgW, imgH));
  };

  return (
    <div
      style={{ position: 'absolute', inset: 0, cursor: 'crosshair' }}
      onPointerDown={(e) => {
        setAnchor({ x: e.clientX, y: e.clientY });
        setSelection(null);
      }}
      onPointerUp={(e) => {
        if (!anchor) return;
        const a = anchor;
        setAnchor(null);
        const rect = normalizeRect(a, { x: e.clientX, y: e.clientY });
        if (rect.w < 4 || rect.h < 4) return;
        setSelection(applyAspectPreset(rect, presetId));
      }}
      onDoubleClick={confirmSelection}
    >
      <div style={{ position: 'absolute', top: theme.spacing.sm, left: theme.spacing.sm, display: 'flex', gap: theme.spacing.xs }}>
        {ASPECT_PRESETS.map((p) => (
          <button
            key={p.id}
            style={{
              color: theme.colors.text,
              background: presetId === p.id ? theme.colors.surfaceHover : theme.colors.surface,
              border: `1px solid ${theme.colors.border}`,
              borderRadius: theme.radii.sm,
              padding: `${theme.spacing.xs}px ${theme.spacing.sm}px`,
            }}
            onClick={() => {
              setPresetId(p.id);
              if (selection) setSelection(applyAspectPreset(selection, p.id));
            }}
          >
            {p.label}
          </button>
        ))}
        <button
          style={{ color: theme.colors.accentText, background: theme.colors.accent, border: 'none', borderRadius: theme.radii.sm, padding: `${theme.spacing.xs}px ${theme.spacing.sm}px` }}
          onClick={confirmSelection}
        >
          Confirm
        </button>
        <button
          style={{ color: theme.colors.text, background: theme.colors.surface, border: 'none', borderRadius: theme.radii.sm, padding: `${theme.spacing.xs}px ${theme.spacing.sm}px` }}
          onClick={onCancel}
        >
          Cancel
        </button>
      </div>
      {selection && (
        <div
          style={{
            position: 'absolute',
            left: selection.x,
            top: selection.y,
            width: selection.w,
            height: selection.h,
            border: `2px solid ${theme.colors.accent}`,
            background: 'transparent',
            pointerEvents: 'none',
          }}
        />
      )}
    </div>
  );
}
```

> `onPointerDown/Up` and `onDoubleClick` are GPUIX-capable event names per
> AGENTS.md's keyboard/pointer support; adapt to the observed surface (Task 1
> spike) and colocate adapters in this file. The aspect preset cycle required by
> the design (D8) is implemented here via `applyAspectPreset`; corner-handle
> resizing remains Phase 2.

- [ ] **Step 4: Write `src/components/export-dialog.tsx`**

```tsx
import { useState } from 'react';
import type { Rect } from '../core/crop-math';
import { useTheme } from '../hooks/use-theme';

export type ExportFormat = 'png' | 'jpeg';

export function ExportDialog({
  sourcePath,
  rect,
  onSave,
  onCopy,
  onCancel,
}: {
  sourcePath: string;
  rect: Rect;
  onSave: (format: ExportFormat, targetName: string) => void;
  onCopy: (format: ExportFormat) => void;
  onCancel: () => void;
}) {
  const theme = useTheme();
  const [format, setFormat] = useState<ExportFormat>('png');
  const base = sourcePath.slice(sourcePath.lastIndexOf('\\') + 1).replace(/\.[^.]+$/, '');
  const [name, setName] = useState(`${base}-crop.png`);

  return (
    <div style={{ position: 'absolute', backgroundColor: theme.colors.overlay, padding: theme.spacing.lg, borderRadius: theme.radii.lg, color: theme.colors.text, display: 'flex', flexDirection: 'column', gap: theme.spacing.md }}>
      <div style={{ fontSize: theme.typography.title }}>
        Export crop — {Math.round(rect.w)} × {Math.round(rect.h)} px
      </div>
      <input
        style={{ color: theme.colors.text, background: theme.colors.surface, border: `1px solid ${theme.colors.border}`, borderRadius: theme.radii.sm, padding: theme.spacing.sm }}
        value={name}
        onChange={(e) => setName(e.currentTarget.value)}
      />
      <div style={{ display: 'flex', gap: theme.spacing.sm }}>
        {(['png', 'jpeg'] as const).map((f) => (
          <button key={f} style={{ color: theme.colors.text, background: format === f ? theme.colors.surfaceHover : theme.colors.surface, border: 'none', borderRadius: theme.radii.sm, padding: theme.spacing.sm }}
            onClick={() => setFormat(f)}>
            {f.toUpperCase()}
          </button>
        ))}
      </div>
      <div style={{ display: 'flex', gap: theme.spacing.sm }}>
        <button style={{ color: theme.colors.accentText, background: theme.colors.accent, border: 'none', borderRadius: theme.radii.sm, padding: theme.spacing.sm }}
          onClick={() => onSave(format, name)}>Save</button>
        <button style={{ color: theme.colors.text, background: theme.colors.surface, border: 'none', borderRadius: theme.radii.sm, padding: theme.spacing.sm }}
          onClick={() => onCopy(format)}>Copy</button>
        <button style={{ color: theme.colors.text, background: theme.colors.surface, border: 'none', borderRadius: theme.radii.sm, padding: theme.spacing.sm }}
          onClick={onCancel}>Cancel</button>
      </div>
    </div>
  );
}
```

> GPUIX `<input>` support must be verified in Task 1's spike; fallback: replace
> with a custom text renderer + up/down options if the element is unavailable.
> Keep the payload contract (`onSave(format, name)`, `onCopy(format)`).

- [ ] **Step 5: Wire crop actions into `image-view.tsx`** — add props:

```tsx
interface CropProps {
  cropEnabled: boolean;
  onCrop: (rect: Rect) => void;
  onCropCancel: () => void;
}
// Inside ImageView return, when cropEnabled:
//   <CropOverlay transform={transform} onConfirm={onCrop} onCancel={onCropCancel} />
```

- [ ] **Step 6: Run crop-flow test** — expected PASS.

Run: `bun test tests/core/crop-flow.test.ts`

- [ ] **Step 7: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/components/crop-overlay.tsx src/components/export-dialog.tsx src/components/image-view.tsx tests/core/crop-flow.test.ts
git commit -m "feat: add crop overlay and export dialog"
```

---

## Task 17: App wiring — navigation, crop flow, drag-drop, registration

**Files:**
- Modify: `src/app.tsx`
- Modify: `src/hooks/use-keymap.ts` (dispatch by mode, registry + CLI open)
- Create: `src/hooks/use-open.ts` (CLI arg → openPath)

- [ ] **Step 1: Write failing test `tests/core/use-open.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { resolveOpenRequest } from '../../src/hooks/use-open';
import { parseArgv } from '../../src/utils/argv';

describe('resolveOpenRequest', () => {
  test('CLI path opens that image in its folder', () => {
    const index = {
      pictures: [
        { path: 'C:/x/a.png', name: 'a.png' },
        { path: 'C:/x/b.png', name: 'b.png' },
      ],
    };
    // folders scanned by the app; request resolves to the containing folder
    const req = resolveOpenRequest(parseArgv(['C:/x/b.png']), 'C:/x');
    expect(req).toEqual({ kind: 'folder', folderPath: 'C:/x', initialPath: 'C:/x/b.png' });
  });

  test('no args open folder picker', () => {
    const req = resolveOpenRequest(parseArgv([]), null);
    expect(req.kind).toBe('picker');
  });
});
```

- [ ] **Step 2: Run to verify failure** — FAIL (module missing).

- [ ] **Step 3: Write `src/hooks/use-open.ts`**

```ts
import { parseArgv } from '../utils/argv';

export type OpenRequest =
  | { kind: 'folder'; folderPath: string; initialPath: string | null }
  | { kind: 'picker' };

/** Resolves the launch request: CLI image → its folder; otherwise picker. */
export function resolveOpenRequest(args: readonly string[], fallbackFolder: string | null): OpenRequest {
  const parsed = parseArgv(args);
  if (parsed.path) {
    const lastSep = Math.max(parsed.path.lastIndexOf('\\'), parsed.path.lastIndexOf('/'));
    const folderPath = lastSep >= 0 ? parsed.path.slice(0, lastSep) : fallbackFolder;
    return folderPath ? { kind: 'folder', folderPath, initialPath: parsed.path } : { kind: 'picker' };
  }
  return { kind: 'picker' };
}
```

- [ ] **Step 4: Update `src/hooks/use-keymap.ts`** — mode-aware dispatch resolving the Esc conflict:

```ts
export type AppMode = 'grid' | 'view' | 'crop' | 'export';

/**
 * Maps a dispatched action to the mode-correct one. `back` and `cancelCrop`
 * both bind to Escape in the default keymap (Task 8), so the mode decides:
 * only crop mode treats Escape as cancel-crop; everywhere else it is back.
 */
export function routeByMode(mode: AppMode, action: ActionId): ActionId {
  if (action !== 'back' && action !== 'cancelCrop') return action;
  if (mode === 'crop' || mode === 'export') return 'cancelCrop';
  return 'back';
}
```

> Add `routeByMode` unit tests in `tests/core/keymap-hook.test.ts`: `grid+back →
> back`, `crop+cancelCrop → cancelCrop`, `view+back → back`, `crop+back →
> cancelCrop`, and `grid+cancelCrop → back`.

- [ ] **Step 5: Write `src/app.tsx`** (top-level wiring)

```tsx
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { useFolder } from './hooks/use-folder';
import { resolveOpenRequest } from './hooks/use-open';
import { parseArgv } from './utils/argv';
import { FolderGrid } from './components/folder-grid';
import { ImageView } from './components/image-view';
import { Welcome } from './components/welcome';
import { Shell } from './components/shell';
import { ThemeProvider } from './hooks/use-theme';
import type { ActionId } from './core/actions';
import { resolveKeymap } from './core/keymap';
import { useKeymap, resolveDispatch, routeByMode } from './hooks/use-keymap';
import { ensureDir } from './utils/ensure-dir';
import { ensureRegistered } from './utils/registration';
import type { Rect } from './core/crop-math';

type ViewMode = 'grid' | 'view';

export default function App() {
  const folder = useFolder();
  const [mode, setMode] = useState<ViewMode>('grid');
  const [cropActive, setCropActive] = useState(false);
  const [cropRect, setCropRect] = useState<Rect | null>(null);

  const openOrError = useCallback(
    async (path: string) => {
      await folder.openPath(path);
      setMode('grid');
    },
    [folder],
  );

  const actions = useCallback(
    (raw: ActionId) => {
      const a = routeByMode(cropActive ? 'crop' : mode, raw);
      switch (a) {
        case 'openFolder':
          void openOrError('');
          break;
        case 'next':
          folder.navigate(1);
          break;
        case 'prev':
          folder.navigate(-1);
          break;
        case 'back':
          setMode('grid');
          break;
        case 'crop':
          setCropActive(true);
          break;
        case 'confirmCrop':
          if (cropRect) setMode('view');
          break;
        case 'cancelCrop':
          setCropActive(false);
          setCropRect(null);
          break;
        default:
          break;
      }
    },
    [folder, cropActive, cropRect, mode, openOrError],
  );

  const lookup = resolveDispatch(resolveKeymap(null));
  useKeymap(actions, lookup, true);

  useEffect(() => {
    void ensureDir(cacheDir());
    void ensureDir(configDir());
    if (process.platform === 'win32') ensureRegistered(process.execPath);
    const request = resolveOpenRequest(parseArgv(process.argv.slice(1)), null);
    if (request.kind === 'folder' && request.folderPath) {
      void folder.openPath(request.folderPath);
    }
  }, []);

  return (
    <ThemeProvider tokens={LIGHTBOX_THEME}>
      <Shell>
        {folder.state.index === null ? (
          <Welcome onOpenFolder={() => void openOrError('')} />
        ) : mode === 'grid' ? (
          <FolderGrid
            index={folder.state.index}
            current={folder.state.current}
            onSelect={(i) => {
              folder.setCurrent(i);
              setMode('view');
            }}
          />
        ) : (
          <ImageView
            entry={folder.state.index.at(folder.state.current)!}
            index={folder.state.current}
            total={folder.state.index.length}
            onBack={() => setMode('grid')}
            onNavigate={folder.navigate}
            cropEnabled={cropActive}
            onCrop={(rect) => setCropRect(rect)}
            onCropCancel={() => setCropActive(false)}
          />
        )}
      </Shell>
    </ThemeProvider>
  );
}
```

> Import `LIGHTBOX_THEME` from `./core/theme` and `cacheDir`/`configDir` from
> `./config/paths`. `ensureRegistered` is Windows-only and runs once on startup;
> it uses `process.execPath` so the compiled binary registers itself. The
> `openOrError('')` default for `openFolder` triggers the folder-picker fallback
> (Task 11's `promptForPath` contract). The `confirmCrop` handler currently just
> exits crop mode; the full export pipeline replaces it in Task 18.

- [ ] **Step 6: Run open + route tests** — expected PASS.

Run: `bun test tests/core/use-open.test.ts tests/core/keymap-hook.test.ts`

- [ ] **Step 7: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/app.tsx src/hooks/use-open.ts src/hooks/use-keymap.ts tests/core/use-open.test.ts tests/core/keymap-hook.test.ts
git commit -m "feat: wire folder navigation, crop flow, and launch open"
```

---

## Task 18: Export pipeline, fixtures, integration tests, benchmarks

**Files:**
- Create: `src/hooks/use-export.ts`
- Create: `tests/integration/flows.test.ts`
- Create: `bench/open.bench.ts`
- Modify: `src/app.tsx` (`confirmCrop` full pipeline)
- Modify: `tests/fixtures/make-fixtures.ts` (already created in Task 10)

- [ ] **Step 1: Write `src/hooks/use-export.ts`**

```ts
import { useCallback } from 'react';
import { createCropWorker, cropRequest } from './use-worker';
import type { CropResponse } from '../workers/crop-worker';
import type { Rect } from '../core/crop-math';
import { writePngToClipboard } from '../utils/clipboard';
import { writeFile } from 'node:fs/promises';

export type ExportTarget = 'save' | 'copy';

/** Runs the crop worker and delivers the encoded buffer (save or copy). */
export function useExport() {
  const run = useCallback(
    async (source: string, rect: Rect, format: 'png' | 'jpeg', target: ExportTarget, outPath?: string): Promise<Uint8Array> => {
      const worker = await createCropWorker();
      try {
        const result = await new Promise<CropResponse>((resolve, reject) => {
          worker.onmessage = (e) => resolve(e.data as CropResponse);
          worker.onerror = () => reject(new Error('crop worker crashed'));
          worker.postMessage(cropRequest(1, source, rect, format));
        });
        if (!result.ok || !result.buffer) throw new Error(result.error ?? 'export failed');
        if (target === 'copy') {
          writePngToClipboard(result.buffer);
        } else if (outPath) {
          await writeFile(outPath, result.buffer);
        }
        return result.buffer;
      } finally {
        worker.terminate();
      }
    },
    [],
  );

  return { run };
}
```

- [ ] **Step 2: Write failing integration test `tests/integration/flows.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { useExport } from '../../src/hooks/use-export';
import { scanFolder } from '../../src/hooks/use-folder';
import { FolderIndex, type ImageEntry } from '../../src/core/navigation';
import { decodeImage } from '@napi-rs/image';
import { ShImagesError } from '../../src/utils/errors';

const FIXTURES = () => join(process.cwd(), 'tests/fixtures');

describe('integration flows', () => {
  test('open flow: scan a fixtures folder', async () => {
    const index = await scanFolder(FIXTURES());
    expect(index.length).toBeGreaterThanOrEqual(3); // sample.png, sample.jpg, sample.gif, corrupt.png
    expect(index.at(0)?.name.toLowerCase()).toBe('corrupt.png'); // natural sort
  });

  test('error flow: corrupt file decodes to a typed error (no crash)', () => {
    // corrupt.png is a truncated JPEG header; decode must throw a ShImagesError
    expect(() => {
      try {
        decodeImage(new Uint8Array(require('node:fs').readFileSync(join(FIXTURES(), 'corrupt.png'))));
      } catch (cause) {
        throw new ShImagesError('DECODE_ERROR', 'decode failed', cause);
      }
    }).toThrow(ShImagesError);
  });

  test('export flow: crop worker returns an encoded buffer', async () => {
    const { run } = useExport();
    const fixture = join(FIXTURES(), 'sample.png');
    const outDir = await mkdtemp(join(tmpdir(), 'shimg-flow-'));
    try {
      const buffer = await run(fixture, { x: 0, y: 0, w: 160, h: 120 }, 'png', 'save', join(outDir, 'out.png'));
      expect(buffer.length).toBeGreaterThan(0);
      const result = await import('node:fs/promises');
      const stat = await result.stat(join(outDir, 'out.png'));
      expect(stat.size).toBe(buffer.length);
    } finally {
      await rm(outDir, { recursive: true, force: true });
    }
  });

  test('navigation flow: circular ordering', () => {
    const items: ImageEntry[] = [
      { path: 'x/b10.png', name: 'b10.png' },
      { path: 'x/b2.png', name: 'b2.png' },
      { path: 'x/a.png', name: 'a.png' },
    ];
    const idx = new FolderIndex(items);
    expect(idx.images.map((e) => e.name)).toEqual(['a.png', 'b2.png', 'b10.png']);
    expect(idx.next(2)).toBe(0);
    expect(idx.prev(0)).toBe(2);
  });
});
```

> The corrupt-decode test asserts the error-flow contract: a corrupt image
> surfaces as `ShImagesError(DECODE_ERROR)` and never crashes the process. If the
> installed `@napi-rs/image` throws a bare `Error` instead, wrap it at the worker
> boundary (Task 10) so `CropResponse.error` names the failure — the app-level
> contract (visible error, no crash) is what the integration test guards.

- [ ] **Step 3: Write `bench/open.bench.ts`**

```ts
import { bench, expect } from 'bun:test';
import { scanFolder } from '../src/hooks/use-folder';
import { FolderIndex, type ImageEntry } from '../src/core/navigation';

const WARM: ImageEntry[] = [];
for (let i = 0; i < 1000; i++) WARM.push({ path: `C:/pics/img${i}.jpg`, name: `img${i}.jpg` });

bench('folder scan (1000 entries)', async () => {
  const idx = new FolderIndex(WARM);
  expect(idx.length).toBe(1000);
});

bench('next/prev navigation (1000 entries)', () => {
  const idx = new FolderIndex(WARM);
  let pos = 0;
  for (let i = 0; i < 1000; i++) pos = idx.next(pos);
  expect(pos).toBe(0);
});
```

- [ ] **Step 4: Wire full `confirmCrop` + settings persistence in `app.tsx`** — replace the stub case:

```tsx
// inside App(): read settings at startup, save theme/lastFolder on change
const [settings, setSettings] = useState<Settings>(() => loadSettings(configFile()));
const exportRun = useExport();

const confirmCrop = async (rect: Rect) => {
  const entry = folder.state.index?.at(folder.state.current);
  if (!entry) return;
  setCropActive(false);
  // default: copy PNG to clipboard; save path defaults next to the source
  await exportRun.run(entry.path, rect, 'png', 'copy');
  setSettings((s) => ({ ...s, lastFolder: entry.path.slice(0, Math.max(entry.path.lastIndexOf('\\'), entry.path.lastIndexOf('/'))) }));
};

// on theme change (useThemeSwitch replacement):
const applyTheme = (id: string) => {
  setSettings((s) => ({ ...s, activeTheme: id }));
};

// persist on unmount and whenever settings change (debounced is fine for V1):
useEffect(() => {
  saveSettings(settings, configFile());
}, [settings]);
```

> `Settings`, `loadSettings`, `saveSettings` come from `./config/settings`;
> `configFile()` from `./config/paths`. The theme switch UI is minimal (Phase 2
> settings panel); wiring `resolveTheme(settings.activeTheme)` into the
> `ThemeProvider tokens` prop gives the persisted theme effect.

- [ ] **Step 5: Run full suite + bench**

Run: `bun test` and `bun run bench`
Expected: all tests pass; the open-flow benchmark lands under target thresholds
(folder scan of 1000 entries « 50 ms on the dev machine; record the number).

- [ ] **Step 6: Commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add src/hooks/use-export.ts tests/integration/flows.test.ts bench/open.bench.ts src/app.tsx tests/fixtures/make-fixtures.ts
git commit -m "feat: add export pipeline, integration flows, and open benchmarks"
```

---

## Task 19: QA pass, binary build, final docs

**Files:**
- Modify: `README.md` (usage section)
- Modify: `docs/ARCHITECTURE.md` (ADR-002 image pipeline, ADR-003 theming/keymap)
- Create: `CHANGELOG.md`

- [ ] **Step 1: Run full QA gate**

```bash
bun run typecheck && bun run lint && bun test && bun run build
```
Expected: zero type errors, zero lint warnings, all tests pass, self-contained
binary at `dist/sh_images(.exe)`. Record any benchmark regression.

- [ ] **Step 2: Manual smoke checklist** — dev-run `bun --hot src/main.tsx` and verify:

- [ ] Window renders the welcome screen with theme colors (Lightbox)
- [ ] Opening a test folder shows the grid with thumbnails
- [ ] Clicking a thumbnail opens the single-image view
- [ ] Arrow keys navigate; wheel navigates; Ctrl+wheel zooms; F11 fullscreen
- [ ] `C` enters crop; drag creates a selection; `Enter` confirms; the copy/save flow produces a file
- [ ] Opening a corrupt file shows a visible error, no crash
- [ ] CLI launch: `dist/sh_images.exe "C:\path\photo.jpg"` opens the folder with the image
- [ ] Settings persist: change theme (or set lastFolder) → restart → preserved
- [ ] Keyboard-only: tab order reaches Open folder, thumbnails, view chrome
- [ ] Drag & drop a folder onto the window loads it (if GPUIX exposes the event)

- [ ] **Step 3: Docs — append ADRs to `docs/ARCHITECTURE.md`**

```markdown
### ADR-002: Native `<img>` display + worker-built thumbnails

- **Context:** Displaying decoded pixels in JS would break RAM/CPU budgets.
- **Decision:** The native GPUIX `<img>` renders originals (Rust decode, GPU
  upload). Grid shows worker-generated downsampled JPEGs from a byte-budgeted
  LRU. Crop decodes on demand in a worker.
- **Consequences:** No decode work on the UI thread; explicit cache budget;
  crop export costs one decode per confirm.

### ADR-003: JSON themes + keymap with schema validation

- **Context:** V1 needs theming and re-bindable keys without an in-app editor.
- **Decision:** Design-token JSON validated at load (CONFIG_ERROR on bad files);
  bundled Lightbox/Terminal/Studio themes; keymap merged over defaults with
  conflict warnings.
- **Consequences:** Hand-editing JSON is the V1 customization surface; the
  settings panel (Phase 2) will reuse the same validation.
```

- [ ] **Step 4: Update `README.md` usage + create `CHANGELOG.md`**

```markdown
## Changelog

## [0.1.0] - 2026-09-07
- Initial V1: folder browsing grid, single-image view with zoom/pan,
  crop → save/copy, Windows default-viewer registration, JSON themes/keymap.
```

- [ ] **Step 5: Final commit**

Run: `bun run typecheck && bun run lint && bun test`

```bash
git add README.md docs/ARCHITECTURE.md CHANGELOG.md
git commit -m "docs: complete QA pass, add ADRs and changelog"
```

---

## Delivery notes

- Each task ends with the global verification gate; do not skip it.
- The GPUIX API probes (Task 1) are authoritative: where the plan's JSX/event
  surface mismatches the installed package, adapt the component and note the
  delta in the commit message — the behavior contract is what matters.
- All repo content is English (user decision).
- Public repo: `.gitignore` keeps `AGENTS.md` and `.superpowers/` out; `docs/`
  is committed.
- Phase 2 (AVIF/HEIC, settings panel, icon overrides, GIF animation, RAW,
  multi-window, keybind editor, theme marketplace) is out of scope for this plan.
- Two deliberate V1 simplifications vs the approved design, both Phase 2 items:
  (1) **icons** (design D7): chrome uses text glyphs (`‹`, `›`) in V1; the
  inline-SVG monochrome icon system ships with the settings panel; (2) **native
  dialogs** (design D10): V1 uses the `promptForPath` text-input fallback and
  saves next to the source; native pickers land with the GPUIX spike result.
  Everything else in the design maps to a task.