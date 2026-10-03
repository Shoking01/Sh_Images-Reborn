#Requires -Version 5.1
<#
.SYNOPSIS
  Windows-only visual-baseline capture harness for Sh_Images.

.DESCRIPTION
  Launches the real release binary against a generated fixture set in a
  hermetic profile, captures each window with `PrintWindow(hwnd, hwnd, 2)`,
  and writes PNGs plus a machine-readable `summary.json`.

  THIS TOOL IS A HUMAN-REVIEWED ARTIFACT, NOT A GATE.

  Pixel output from a composited GPU window is not reproducible across
  machines: it varies with GPU driver, Windows build, DPI scale and font
  fallback. A golden-image comparison therefore reports a delta and never
  fails a build. See docs/ARCHITECTURE.md ADR-022 for why CI gating stays on
  geometry invariants and this stays advisory.

  Three guards exist because a wrong frame is worse than no frame. A human
  reviewing a screenshot calls a broken half-painted image a regression, and a
  CI runner with no compositor produces solid black frames that look like
  valid artifacts:

    1. Stability gate  - a frame is accepted only after two CONSECUTIVE
       captures are pixel-identical, so a mid-load frame (thumbnails still
       decoding, half the window painted) is never written.
    2. Blank detection - a frame whose sampled pixels are essentially uniform
       is a capture FAILURE and is never written as an artifact.
    3. Process hygiene - the launched process is killed in a `finally`, on
       success and on every error path alike.

.PARAMETER OutputDir
  Directory that receives the PNGs, `compare/` and `summary.json`.
  Defaults to `artifacts/visual/current` under the repository root.

.PARAMETER ExePath
  Path to `sh-app.exe`. Defaults to `target/release/sh-app.exe` under the
  repository root.

.PARAMETER Compare
  Compare the captures against the committed baselines in
  `artifacts/visual/baseline/`, writing diff PNGs and a per-image pixel-delta
  percentage. Never affects the exit code.

.PARAMETER BaselineDir
  Baseline directory used by -Compare. Defaults to
  `artifacts/visual/baseline` under the repository root.

.EXAMPLE
  .\capture.ps1
  Capture every scenario into artifacts/visual/current.

.EXAMPLE
  .\capture.ps1 -Compare
  Capture, then report the pixel delta against the committed baselines.

.NOTES
  Exit codes:
    0  at least one scenario produced a valid frame (and -Compare ran).
    1  total capture failure - nothing valid was produced, or the harness
       itself failed. Pixel delta NEVER produces this.
#>
[CmdletBinding()]
param(
    [string] $OutputDir,
    [string] $ExePath,
    [string] $BaselineDir,
    [switch] $Compare,
    [int] $MaxStabilityAttempts = 12,
    [int] $InitialSettleMs = 1500,
    [int] $InterAttemptDelayMs = 600,
    [int] $WindowTimeoutSec = 45,
    [switch] $KeepRunRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# --------------------------------------------------------------------------
# Native interop. Pixel hashing, blank detection and diffing run in C# rather
# than in PowerShell loops: a 1000x720 frame is 720k pixels and a PS loop over
# that is minutes per comparison.
# --------------------------------------------------------------------------
if (-not ('ShImagesVisual.Win32' -as [type])) {
    Add-Type -ReferencedAssemblies 'System.Drawing' -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;

namespace ShImagesVisual
{
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    public struct POINT { public int X; public int Y; }

    public static class Win32
    {
        // nFlags 2 == PW_RENDERFULLCONTENT. PW_CLIENTONLY (1) and flag 0 both
        // return an empty frame for a composited GPU window here.
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdcBlt, uint nFlags);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);

        [DllImport("user32.dll")]
        public static extern bool SetProcessDPIAware();

        [DllImport("user32.dll")]
        public static extern bool IsWindowVisible(IntPtr hwnd);

        [DllImport("user32.dll")]
        public static extern bool ShowWindow(IntPtr hwnd, int nCmdShow);

        // Pointer control. See ParkPointer / the harness notes: an unhovered
        // window is the only way to get a frame that does not depend on where
        // the developer's hand happened to be.
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool GetCursorPos(out POINT point);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool SetCursorPos(int x, int y);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool GetClientRect(IntPtr hwnd, out RECT rect);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool ClientToScreen(IntPtr hwnd, ref POINT point);

        /// <summary>
        /// Saves the pointer position so the harness can put it back. Moving a
        /// developer's cursor and leaving it in a corner is rude, and on a
        /// remote session it is the kind of thing that gets noticed.
        /// </summary>
        public static POINT SaveCursor()
        {
            POINT p;
            GetCursorPos(out p);
            return p;
        }

        /// <summary>
        /// Parks the pointer in the top-left of the virtual desktop, outside any
        /// window this harness opens, and restores it afterwards.
        /// </summary>
        public static bool ParkPointer()
        {
            return SetCursorPos(0, 0);
        }

        public static bool RestoreCursor(POINT saved)
        {
            return SetCursorPos(saved.X, saved.Y);
        }

        /// <summary>
        /// Rows of the captured bitmap that sit ABOVE the client area, i.e. the
        /// native title bar and borders, which Windows draws rather than the app.
        ///
        /// This exists because that band is not comparable and cannot be made
        /// comparable. A focused window paints a warm gradient title bar and an
        /// unfocused one paints flat #202020: 31252 differing pixels, 4.05% of a
        /// 1016x759 frame, for a difference the app did not make. Forcing the
        /// window to the foreground was tried and rejected: Windows' foreground
        /// lock refuses a background process outright, and even borrowing the
        /// foreground thread's input queue failed on a desktop in active use.
        /// Stealing a developer's focus four times per run is also rude.
        ///
        /// So the band is measured, not assumed - GetClientRect plus
        /// ClientToScreen give the real offset for the current window, DPI and
        /// border style included - and it is excluded from the delta instead.
        /// Returns 0 when the client area starts at the top, which is what a
        /// maximized borderless window reports.
        /// </summary>
        public static int ClientTopOffset(IntPtr hwnd)
        {
            RECT wr;
            if (!GetWindowRect(hwnd, out wr)) { return 0; }

            RECT cr;
            if (!GetClientRect(hwnd, out cr)) { return 0; }

            POINT origin = new POINT();
            origin.X = cr.Left;
            origin.Y = cr.Top;
            if (!ClientToScreen(hwnd, ref origin)) { return 0; }

            int offset = origin.Y - wr.Top;
            return offset > 0 ? offset : 0;
        }
    }

    public sealed class FrameStats
    {
        public int Width;
        public int Height;
        public int DistinctSampledColors;
        public double DominantColorShare;
        public long LatticeSamples;
        public int LatticeStep;
        public string PixelHash;
        public bool IsUniform;
        public string UniformReason;
    }

    public sealed class CompareResult
    {
        public string Status;
        public string Note;
        public double DeltaPercent;
        public long DifferingPixels;
        public long TotalPixels;
        public int Tolerance;
        public int BaselineWidth;
        public int BaselineHeight;
        public int CurrentWidth;
        public int CurrentHeight;
        public string DiffPath;
        public int IgnoredTopRows;
        public long ComparedPixels;
    }

    public static class Frames
    {
        static int[] Argb(Bitmap bmp)
        {
            int w = bmp.Width, h = bmp.Height;
            var rect = new Rectangle(0, 0, w, h);
            var data = bmp.LockBits(rect, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            try
            {
                int stride = data.Stride;
                if (stride < 0)
                {
                    throw new InvalidOperationException(
                        "negative bitmap stride " + stride + "; byte indexing would be wrong");
                }
                var buf = new byte[stride * h];
                Marshal.Copy(data.Scan0, buf, 0, buf.Length);
                var px = new int[w * h];
                for (int y = 0; y < h; y++)
                {
                    int row = y * stride;
                    int dst = y * w;
                    for (int x = 0; x < w; x++)
                    {
                        int i = row + x * 4;
                        // 32bpp ARGB is stored B, G, R, A in memory.
                        int b = buf[i];
                        int g = buf[i + 1];
                        int r = buf[i + 2];
                        px[dst + x] = (r << 16) | (g << 8) | b;
                    }
                }
                return px;
            }
            finally { bmp.UnlockBits(data); }
        }

        // FNV-1a over the raw pixels. Not a cryptographic hash: it only has to
        // decide "are these two frames the same frame".
        static string HashOf(int[] px)
        {
            ulong h = 14695981039346656037UL;
            for (int i = 0; i < px.Length; i++)
            {
                int v = px[i];
                h ^= (ulong)(v & 0xFF);      h *= 1099511628211UL;
                h ^= (ulong)((v >> 8) & 0xFF);   h *= 1099511628211UL;
                h ^= (ulong)((v >> 16) & 0xFF);  h *= 1099511628211UL;
            }
            return h.ToString("x16");
        }

        public static Bitmap ToBgra32(Bitmap source)
        {
            var dst = new Bitmap(source.Width, source.Height, PixelFormat.Format32bppArgb);
            using (var g = Graphics.FromImage(dst))
            {
                g.CompositingMode = CompositingMode.SourceCopy;
                g.DrawImage(source, new Rectangle(0, 0, dst.Width, dst.Height));
            }
            return dst;
        }

        public static Bitmap LoadPng(string path)
        {
            using (var src = new Bitmap(path)) { return ToBgra32(src); }
        }

        public static void SavePng(Bitmap bmp, string path)
        {
            using (var flat = ToBgra32(bmp)) { flat.Save(path, ImageFormat.Png); }
        }

        /// <summary>
        /// Samples a ~128-per-axis lattice rather than every pixel, and reports
        /// the distinct-color count plus the share of the most common color.
        /// A real frame has a large background region but never approaches
        /// 99.5% of one color; a compositor failure is uniformly one color.
        /// </summary>
        public static FrameStats Analyze(Bitmap bmp)
        {
            var st = new FrameStats();
            st.Width = bmp.Width;
            st.Height = bmp.Height;
            int[] px = Argb(bmp);
            st.PixelHash = HashOf(px);

            int step = Math.Max(1, Math.Max(bmp.Width, bmp.Height) / 128);
            st.LatticeStep = step;
            var counts = new Dictionary<int, int>();
            long samples = 0;
            int dominant = 0;
            for (int y = 0; y < bmp.Height; y += step)
            {
                int row = y * bmp.Width;
                for (int x = 0; x < bmp.Width; x += step)
                {
                    int c = px[row + x];
                    int n;
                    counts.TryGetValue(c, out n);
                    n++;
                    counts[c] = n;
                    if (n > dominant) { dominant = n; }
                    samples++;
                }
            }
            st.LatticeSamples = samples;
            st.DistinctSampledColors = counts.Count;
            st.DominantColorShare = samples == 0 ? 1.0 : (double)dominant / samples;

            if (st.DistinctSampledColors <= 1)
            {
                string only = "000000";
                foreach (var kv in counts) { only = kv.Key.ToString("x6"); break; }
                st.IsUniform = true;
                st.UniformReason = "frame is a single uniform color (#" + only + ") over " +
                    samples + " sampled pixels";
            }
            else if (st.DominantColorShare >= 0.995)
            {
                st.IsUniform = true;
                st.UniformReason = "frame is essentially uniform: one color covers " +
                    (st.DominantColorShare * 100.0).ToString("F2") + "% of " + samples + " sampled pixels";
            }
            return st;
        }

        public static CompareResult Compare(string baselinePath, string currentPath, int tolerance, string diffPath, int ignoreTopRows)
        {
            var r = new CompareResult();
            r.Tolerance = tolerance;
            r.DeltaPercent = -1.0;
            r.Status = "error";
            r.IgnoredTopRows = ignoreTopRows;

            Bitmap b, c;
            try { b = LoadPng(baselinePath); }
            catch (Exception e) { r.Note = "baseline unreadable: " + e.Message; return r; }
            try { c = LoadPng(currentPath); }
            catch (Exception e) { b.Dispose(); r.Note = "current unreadable: " + e.Message; return r; }

            r.BaselineWidth = b.Width; r.BaselineHeight = b.Height;
            r.CurrentWidth = c.Width;    r.CurrentHeight = c.Height;

            int w = Math.Max(b.Width, c.Width);
            int h = Math.Max(b.Height, c.Height);
            var diff = new Bitmap(w, h, PixelFormat.Format32bppArgb);

            int[] pb = Argb(b);
            int[] pc = Argb(c);

            using (var g = Graphics.FromImage(diff))
            {
                // Regions present in only one of the two frames are painted
                // magenta. A size difference is a portability fact, not a
                // regression, and it must be impossible to miss in the diff.
                using (var brush = new SolidBrush(Color.FromArgb(255, 255, 0, 255)))
                {
                    if (b.Width != w || b.Height != h) { g.FillRectangle(brush, 0, 0, w, h); }
                    if (c.Width != w || c.Height != h) { g.FillRectangle(brush, 0, 0, c.Width, c.Height); }
                }
            }

            long total = (long)w * h;
            long compared = 0;
            long differing = 0;
            for (int y = 0; y < h; y++)
            {
                if (y < ignoreTopRows)
                {
                    // Not compared: see Win32.ClientTopOffset. Marked in the
                    // diff in a flat blue so a reviewer can see at a glance
                    // that this band was excluded rather than identical.
                    for (int x = 0; x < w; x++)
                    {
                        diff.SetPixel(x, y, Color.FromArgb(255, 0, 0, 96));
                    }
                    continue;
                }
                compared += w;
                for (int x = 0; x < w; x++)
                {
                    int vb = (x < b.Width && y < b.Height) ? pb[y * b.Width + x] : -1;
                    int vc = (x < c.Width && y < c.Height) ? pc[y * c.Width + x] : -1;
                    int d;
                    if (vb < 0 || vc < 0)
                    {
                        d = 255;
                    }
                    else
                    {
                        int dr = Math.Abs(((vb >> 16) & 0xFF) - ((vc >> 16) & 0xFF));
                        int dg = Math.Abs(((vb >> 8) & 0xFF) - ((vc >> 8) & 0xFF));
                        int db = Math.Abs((vb & 0xFF) - (vc & 0xFF));
                        d = Math.Max(dr, Math.Max(dg, db));
                    }
                    if (d <= tolerance) { continue; }
                    differing++;
                    // Identical pixels go black, differing ones go red/yellow
                    // by magnitude so a faint anti-aliasing shift stays
                    // distinguishable from a whole element moving.
                    int mag = Math.Min(255, d);
                    diff.SetPixel(x, y, Color.FromArgb(255, mag, (byte)Math.Min(255, mag / 4), 0));
                }
            }

            r.DifferingPixels = differing;
            r.TotalPixels = total;
            r.ComparedPixels = compared;
            r.DiffPath = diffPath;

            if (b.Width != c.Width || b.Height != c.Height)
            {
                r.Status = "dimension_mismatch";
                r.DeltaPercent = 100.0;
                r.Note = "frame size differs (baseline " + b.Width + "x" + b.Height +
                    ", current " + c.Width + "x" + c.Height +
                    "); magenta marks the non-overlapping area. Expected across" +
                    " machines: window borders, DPI scale and Windows build all" +
                    " change the captured rect. Not a regression by itself.";
            }
            else
            {
                r.DeltaPercent = compared == 0 ? 0.0 : (double)differing * 100.0 / compared;
                r.Status = differing == 0 ? "identical" : "different";
                r.Note = "tolerance " + tolerance + " per channel; " + differing + " of " +
                    compared + " compared pixels differ";
                if (ignoreTopRows > 0)
                {
                    r.Note += " (top " + ignoreTopRows + " row(s) of OS-drawn window chrome" +
                        " excluded; blue band in the diff)";
                }
            }

            diff.Save(diffPath, ImageFormat.Png);
            diff.Dispose();
            b.Dispose();
            c.Dispose();
            return r;
        }
    }

    public sealed class FixtureSpec
    {
        public string Name;
        public int Width;
        public int Height;
        public int R;
        public int G;
        public int B;
    }

    public static class Fixtures
    {
        // Aspect ratios cycle wide -> square -> tall, so ANY consecutive run of
        // items contains a mix. That matters because the defect this harness
        // exists for is a tall cell inflating its own flex line: it only shows
        // when a tall image sits next to shorter ones in the same row.
        public static FixtureSpec[] Specs()
        {
            return new FixtureSpec[]
            {
                new FixtureSpec { Name = "01-wide-0800x0200",   Width = 800, Height = 200, R = 0xD6, G = 0x45, B = 0x45 },
                new FixtureSpec { Name = "02-square-0400x0400", Width = 400, Height = 400, R = 0x45, G = 0xA0, B = 0xD6 },
                new FixtureSpec { Name = "03-tall-0200x0800",   Width = 200, Height = 800, R = 0x4F, G = 0xBF, B = 0x6A },
                new FixtureSpec { Name = "04-wide-0800x0200",   Width = 800, Height = 200, R = 0xE0, G = 0xA9, B = 0x3C },
                new FixtureSpec { Name = "05-square-0400x0400", Width = 400, Height = 400, R = 0x9B, G = 0x5F, B = 0xD6 },
                new FixtureSpec { Name = "06-tall-0200x0800",   Width = 200, Height = 800, R = 0x38, G = 0xC4, B = 0xC4 },
            };
        }

        public static string[] Create(string dir)
        {
            Directory.CreateDirectory(dir);
            var written = new List<string>();
            foreach (var s in Specs())
            {
                string path = Path.Combine(dir, s.Name + ".png");
                // 24bpp: no alpha channel, so the encoder emits no ancillary
                // chunks that could vary between runs.
                using (var bmp = new Bitmap(s.Width, s.Height, PixelFormat.Format24bppRgb))
                {
                    using (var g = Graphics.FromImage(bmp))
                    {
                        g.Clear(Color.FromArgb(255, s.R, s.G, s.B));
                    }
                    bmp.Save(path, ImageFormat.Png);
                }
                written.Add(path);
            }
            return written.ToArray();
        }
    }
}
'@
}

# --------------------------------------------------------------------------
# Defaults, resolved from the script location so the tool works from any cwd.
# --------------------------------------------------------------------------
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $OutputDir)   { $OutputDir   = Join-Path $repoRoot 'artifacts\visual\current' }
if (-not $ExePath)     { $ExePath     = Join-Path $repoRoot 'target\release\sh-app.exe' }
if (-not $BaselineDir) { $BaselineDir = Join-Path $repoRoot 'artifacts\visual\baseline' }

# A per-channel tolerance below which a difference is treated as noise. Text
# antialiasing and gradient dithering shift by a unit or two between captures
# on identical hardware, and calling that a delta would make the number
# meaningless. Anything a human would point at is far above this.
$DeltaTolerance = 8
$BlankDominantShare = 0.995

function Write-Log {
    param([string] $Message, [string] $Level = 'info')
    Write-Host ("[{0}] {1}" -f $Level.ToUpperInvariant(), $Message)
}

function Fail {
    param([string] $Message)
    throw $Message
}

# --------------------------------------------------------------------------
# Interop environment notes.
# --------------------------------------------------------------------------
$dpiAware = $false
try { $dpiAware = [ShImagesVisual.Win32]::SetProcessDPIAware() } catch {
    Write-Log ("could not set DPI awareness: {0}" -f $_.Exception.Message) 'warn'
}
Add-Type -AssemblyName System.Drawing | Out-Null

# --------------------------------------------------------------------------
# Output / run directories.
# --------------------------------------------------------------------------
$resolvedOut = [System.IO.Path]::GetFullPath($OutputDir)
$resolvedBaseGuard = [System.IO.Path]::GetFullPath($BaselineDir)
# The harness clears stale PNGs from the output directory so a failed re-run
# cannot leave a previous run's frame sitting there looking current. That
# makes the baseline directory a destructive target, so refuse it outright.
if ($resolvedOut.TrimEnd('\') -eq $resolvedBaseGuard.TrimEnd('\')) {
    Fail ("refusing to use the baseline directory as the output directory: {0}" -f $resolvedBaseGuard)
}
if (Test-Path -LiteralPath $resolvedOut) {
    Get-ChildItem -LiteralPath $resolvedOut -Filter '*.png' -File -ErrorAction SilentlyContinue |
        Remove-Item -Force
} else {
    New-Item -ItemType Directory -Path $resolvedOut -Force | Out-Null
}
$compareDir = Join-Path $resolvedOut 'compare'
if (Test-Path -LiteralPath $compareDir) {
    Get-ChildItem -LiteralPath $compareDir -File -ErrorAction SilentlyContinue | Remove-Item -Force
} else {
    New-Item -ItemType Directory -Path $compareDir -Force | Out-Null
}

# One run root holds both the fixtures and the hermetic profile, so a failed
# run leaves everything needed to reproduce it behind a single printed path.
$runRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("sh-images-visual-{0}" -f [Guid]::NewGuid().ToString('n'))
$fixtureDir = Join-Path $runRoot 'fixtures'
$profileDir = Join-Path $runRoot 'profile'
New-Item -ItemType Directory -Path $fixtureDir -Force | Out-Null
New-Item -ItemType Directory -Path $profileDir -Force | Out-Null
$emptyDir = Join-Path $runRoot 'empty'
New-Item -ItemType Directory -Path $emptyDir -Force | Out-Null

$script:Launched = New-Object System.Collections.ArrayList
# Captured once, restored once per scenario, so a run leaves the desktop's
# pointer exactly where the developer left it.
$savedCursor = [ShImagesVisual.Win32]::SaveCursor()
$script:SavedCursor = $savedCursor
Write-Log ("pointer parked-for-capture origin: {0},{1}" -f $savedCursor.X, $savedCursor.Y)
$scenarioResults = New-Object System.Collections.ArrayList
$fixtureDigests = @()
$exeFull = [System.IO.Path]::GetFullPath($ExePath)
$fatal = $null

try {
    # ----------------------------------------------------------------------
    # Preconditions.
    # ----------------------------------------------------------------------
    if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
        Fail ("executable not found: {0}`nBuild it first: cargo build --release" -f $ExePath)
    }
    $exeFull = (Resolve-Path -LiteralPath $ExePath).Path
    Write-Log ("exe: {0}" -f $exeFull)
    Write-Log ("output: {0}" -f $resolvedOut)
    Write-Log ("dpi aware: {0}" -f $dpiAware)

    # ----------------------------------------------------------------------
    # Fixtures. Regenerated every run; never hand-authored.
    # ----------------------------------------------------------------------
    $fixtureFiles = [ShImagesVisual.Fixtures]::Create($fixtureDir)
    $fixtureDigests = @()
    foreach ($f in $fixtureFiles) {
        $fixtureDigests += [pscustomobject]@{
            file = [System.IO.Path]::GetFileName($f)
            bytes = (Get-Item -LiteralPath $f).Length
        }
    }
    Write-Log ("fixtures: {0} images in {1}" -f $fixtureFiles.Count, $fixtureDir)

    # The viewer scenario opens one file. The square one is chosen over the
    # wide or the tall one because it is the only shape that exercises all
    # four letterbox edges at once.
    $viewerFixture = Join-Path $fixtureDir '02-square-0400x0400.png'

    # ----------------------------------------------------------------------
    # Scenarios. `args` is the raw argv tail handed to the child process.
    # ----------------------------------------------------------------------
    $scenarios = @(
        [pscustomobject]@{ id = 'grid-mixed-aspect'; description = 'Grid view over the mixed-aspect fixture set'; args = ('"{0}"' -f $fixtureDir) }
        [pscustomobject]@{ id = 'viewer-single';     description = 'Viewer view on a single square fixture';        args = ('"{0}"' -f $viewerFixture) }
        [pscustomobject]@{ id = 'welcome';            description = 'Welcome view, no CLI argument';                  args = '' }
        [pscustomobject]@{ id = 'empty-grid';        description = 'Grid view over an empty directory';              args = ('"{0}"' -f $emptyDir) }
    )

    foreach ($scenario in $scenarios) {
        Write-Log ("--- scenario {0}: {1}" -f $scenario.id, $scenario.description)
        $result = [ordered]@{
            id                = $scenario.id
            description       = $scenario.description
            args              = $scenario.args
            status            = 'failed'
            reason            = ''
            png               = $null
            width             = 0
            height            = 0
            distinctColors    = 0
            dominantShare     = 0.0
            pixelHash         = ''
            attempts          = 0
            stableAfterAttempt = 0
            observedHashes    = @()
            sawBlankFrame     = $false
            parkedPointer     = $false
            ignoredTopRows    = 0
        }

        $proc = $null
        try {
            # ---- launch -------------------------------------------------
            $psi = New-Object System.Diagnostics.ProcessStartInfo
            $psi.FileName = $exeFull
            $psi.Arguments = $scenario.args
            $psi.UseShellExecute = $false
            $psi.CreateNoWindow = $true
            $psi.WorkingDirectory = Split-Path -Parent $exeFull
            # Hermetic profile. config_dir() resolves to %APPDATA%\sh_images,
            # so pointing APPDATA at a fresh directory gives the app its
            # own bootstrapped defaults and theme without reading or writing
            # the developer's real profile.
            #
            # settings.json is deliberately NOT hand-authored: the app
            # bootstraps current defaults into a missing profile, so the
            # defaults are the deterministic input. A hand-written file would
            # silently drift the day the schema version moves.
            $psi.EnvironmentVariables['APPDATA'] = $profileDir
            # LOCALAPPDATA is left alone on purpose: it holds OS and GPU
            # caches, and overriding it changes rendering behaviour rather
            # than isolating app config.

            $proc = New-Object System.Diagnostics.Process
            $proc.StartInfo = $psi
            if (-not $proc.Start()) { Fail 'child process refused to start' }
            [void]$script:Launched.Add($proc)
            Write-Log ("  pid {0}" -f $proc.Id)

            # ---- wait for a real top-level window ----------------------
            $deadline = (Get-Date).AddSeconds($WindowTimeoutSec)
            $hwnd = [IntPtr]::Zero
            while ((Get-Date) -lt $deadline) {
                if ($proc.HasExited) {
                    Fail ("child exited early with code {0}" -f $proc.ExitCode)
                }
                $proc.Refresh()
                if ($proc.MainWindowHandle -ne [IntPtr]::Zero) {
                    $hwnd = $proc.MainWindowHandle
                    break
                }
                Start-Sleep -Milliseconds 200
            }
            if ($hwnd -eq [IntPtr]::Zero) {
                Fail ("no top-level window appeared within {0}s" -f $WindowTimeoutSec)
            }
            $rect = New-Object ShImagesVisual.RECT
            if (-not [ShImagesVisual.Win32]::GetWindowRect($hwnd, [ref] $rect)) {
                Fail 'GetWindowRect failed'
            }
            $winW = $rect.Right - $rect.Left
            $winH = $rect.Bottom - $rect.Top
            Write-Log ("  hwnd {0} rect {1}x{2}" -f $hwnd, $winW, $winH)

            # A minimized window renders nothing through PrintWindow. The app
            # opens centered and visible, so this is only a guard against an
            # environment that starts it minimized.
            try { [void][ShImagesVisual.Win32]::ShowWindow($hwnd, 5) } catch {
                Write-Log ("  ShowWindow failed: {0}" -f $_.Exception.Message) 'warn'
            }

            Start-Sleep -Milliseconds $InitialSettleMs

            # ---- deterministic input state -------------------------------
            # Two pieces of uncontrolled input reach the renderer, and both show
            # up in the pixels. This is why an earlier revision of this harness
            # produced two different "stable" frames for identical code:
            #
            #   * The mouse pointer, which is pinned below. A hovered grid cell
            #     paints an elevated card 180px wide against the 160px that
            #     every other cell measures, so a frame captured with the
            #     pointer resting on a cell is not a different APP state, it is
            #     a different MOUSE state - and a human reading that diff sees a
            #     cell that grew and calls it a regression.
            #   * Window activation, which is NOT pinned - it cannot be. Windows'
            #     foreground lock refuses a background process, and borrowing the
            #     foreground thread's input queue still failed on a desktop in
            #     active use. Forcing focus four times a run would also steal the
            #     developer's focus. Instead the OS-drawn band above the client
            #     area is measured and excluded from the delta; see
            #     Win32.ClientTopOffset.
            $parked = $false
            try {
                $parked = [ShImagesVisual.Win32]::ParkPointer()
            } catch {
                Write-Log ("  ParkPointer failed: {0}" -f $_.Exception.Message) 'warn'
            }
            if ($parked) {
                # Let the hover state clear before the first sample, otherwise
                # the first frame legitimately differs from the rest and the
                # gate burns an attempt on it.
                Start-Sleep -Milliseconds 300
            }
            $clientTop = 0
            try { $clientTop = [ShImagesVisual.Win32]::ClientTopOffset($hwnd) } catch {
                Write-Log ("  ClientTopOffset failed: {0}" -f $_.Exception.Message) 'warn'
            }
            $result.parkedPointer = $parked
            $result.ignoredTopRows = $clientTop
            Write-Log ("  pointer parked: {0}; native chrome excluded from delta: top {1} row(s)" -f $parked, $clientTop)

            # ---- stability gate ----------------------------------------
            $previousHash = $null
            $accepted = $null
            $hashes = @()
            $blankReason = ''
            $lastReason = ''

            for ($attempt = 1; $attempt -le $MaxStabilityAttempts; $attempt++) {
                $result.attempts = $attempt
                if ($proc.HasExited) {
                    $lastReason = ("child exited during capture (code {0})" -f $proc.ExitCode)
                    break
                }

                $bmp = $null
                try {
                    $live = New-Object ShImagesVisual.RECT
                    if (-not [ShImagesVisual.Win32]::GetWindowRect($hwnd, [ref] $live)) {
                        $lastReason = 'GetWindowRect failed mid-capture'
                        break
                    }
                    $w = $live.Right - $live.Left
                    $h = $live.Bottom - $live.Top
                    if ($w -lt 8 -or $h -lt 8) {
                        $lastReason = ("degenerate window rect {0}x{1}" -f $w, $h)
                        break
                    }

                    $bmp = New-Object System.Drawing.Bitmap($w, $h, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
                    $g = [System.Drawing.Graphics]::FromImage($bmp)
                    try {
                        $hdc = $g.GetHdc()
                        try {
                            # Flag 2 == PW_RENDERFULLCONTENT.
                            [void][ShImagesVisual.Win32]::PrintWindow($hwnd, $hdc, 2)
                        } finally {
                            $g.ReleaseHdc($hdc)
                        }
                    } finally {
                        $g.Dispose()
                    }
                } catch {
                    $lastReason = ("capture failed: {0}" -f $_.Exception.Message)
                    break
                }

                $stats = $null
                try { $stats = [ShImagesVisual.Frames]::Analyze($bmp) }
                finally { $bmp.Dispose() }

                $hashes += $stats.PixelHash
                $result.sawBlankFrame = [bool]($result.sawBlankFrame -or $stats.IsUniform)
                if ($stats.IsUniform) { $blankReason = $stats.UniformReason }

                if ($stats.IsUniform) {
                    Write-Log ("  attempt {0}/{1}: {2}" -f $attempt, $MaxStabilityAttempts, $stats.UniformReason) 'warn'
                } elseif ($stats.PixelHash -eq $previousHash) {
                    Write-Log ("  attempt {0}/{1}: stable, {2} distinct colors, {3:P2} dominant" -f `
                        $attempt, $MaxStabilityAttempts, $stats.DistinctSampledColors, $stats.DominantColorShare)
                    $accepted = $stats
                    $result.stableAfterAttempt = $attempt
                    break
                } else {
                    Write-Log ("  attempt {0}/{1}: still changing, {2} distinct colors" -f `
                        $attempt, $MaxStabilityAttempts, $stats.DistinctSampledColors)
                }

                $previousHash = $stats.PixelHash
                Start-Sleep -Milliseconds $InterAttemptDelayMs
            }
            $result.observedHashes = $hashes

            if ($null -ne $accepted) {
                $pngPath = Join-Path $resolvedOut ($scenario.id + '.png')
                # Recapture only if needed: the accepted frame was already
                # disposed, so take one more and re-verify it is still the
                # frame that was accepted. If it is not, the window changed
                # after acceptance and this artifact would be a lie.
                $saveBmp = New-Object System.Drawing.Bitmap($accepted.Width, $accepted.Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
                try {
                    $g2 = [System.Drawing.Graphics]::FromImage($saveBmp)
                    try {
                        $hdc2 = $g2.GetHdc()
                        try { [void][ShImagesVisual.Win32]::PrintWindow($hwnd, $hdc2, 2) }
                        finally { $g2.ReleaseHdc($hdc2) }
                    } finally { $g2.Dispose() }
                    $recheck = [ShImagesVisual.Frames]::Analyze($saveBmp)
                    if ($recheck.IsUniform) {
                        $saveBmp.Dispose()
                        $result.status = 'failed'
                        $result.reason = ("frame became uniform between the stability gate and the save: {0}" -f $recheck.UniformReason)
                    } elseif ($recheck.PixelHash -ne $accepted.PixelHash) {
                        $saveBmp.Dispose()
                        $result.status = 'failed'
                        $result.reason = ('window changed after the stability gate; refusing to save a frame that is not the one that was verified')
                    } else {
                        [ShImagesVisual.Frames]::SavePng($saveBmp, $pngPath)
                        $result.status = 'captured'
                        $result.png = [System.IO.Path]::GetFileName($pngPath)
                        $result.width = $accepted.Width
                        $result.height = $accepted.Height
                        $result.distinctColors = $accepted.DistinctSampledColors
                        $result.dominantShare = [Math]::Round($accepted.DominantColorShare, 4)
                        $result.pixelHash = $accepted.PixelHash
                        Write-Log ("  wrote {0} ({1}x{2})" -f $pngPath, $accepted.Width, $accepted.Height)
                    }
                } finally {
                    if ($saveBmp) { $saveBmp.Dispose() }
                }
            } else {
                $result.status = 'failed'
                if ($blankReason) {
                    $result.reason = ("no usable frame: every capture was blank ({0}). The window was" -f $blankReason) +
                        ' never composited - typical on a session with no interactive desktop' +
                        ' (headless CI, RDP disconnect, locked workstation).'
                } else {
                    $result.reason = ("stability never reached after {0} attempts ({1}); the frame was" -f $MaxStabilityAttempts, $lastReason) +
                        ' still changing every time, which usually means decoding or animation never settles.'
                }
                Write-Log ("  FAILED: {0}" -f $result.reason) 'error'
            }
        } catch {
            $result.status = 'failed'
            $result.reason = $_.Exception.Message
            Write-Log ("  FAILED: {0}" -f $_.Exception.Message) 'error'
        } finally {
            # Process hygiene, on success and on every error path.
            if ($script:SavedCursor) {
                try { [void][ShImagesVisual.Win32]::RestoreCursor($script:SavedCursor) } catch { }
            }
            if ($proc) {
                if (-not $proc.HasExited) {
                    Write-Log ("  killing pid {0}" -f $proc.Id) 'warn'
                    try { $proc.Kill() } catch { Write-Log ("  Kill failed: {0}" -f $_.Exception.Message) 'warn' }
                }
                try { [void]$proc.WaitForExit(10000) } catch { }
                [void]$script:Launched.Remove($proc)
                try { $proc.Dispose() } catch { }
            }
            Start-Sleep -Milliseconds 300
        }

        [void]$scenarioResults.Add([pscustomobject]$result)
    }
} catch {
    $fatal = $_.Exception.Message
    Write-Log ("harness failure: {0}" -f $fatal) 'error'
} finally {
    # Backstop: nothing this script started may outlive it.
    foreach ($p in @($script:Launched)) {
        if ($p -and -not $p.HasExited) {
            try { $p.Kill() } catch { }
        }
    }
    $script:Launched.Clear()
}

# --------------------------------------------------------------------------
# Compare mode. Reports a delta. Never affects the exit code.
# --------------------------------------------------------------------------
$compareResults = @()
if ($Compare) {
    Write-Log '--- compare against committed baselines'
    $resolvedBase = [System.IO.Path]::GetFullPath($BaselineDir)
    if (-not (Test-Path -LiteralPath $resolvedBase -PathType Container)) {
        Write-Log ("baseline directory not found: {0}" -f $resolvedBase) 'error'
        $compareResults += [pscustomobject]@{
            scenario = '*'; status = 'no_baseline_dir'; note = $resolvedBase
            deltaPercent = $null; diffPng = $null
        }
    } else {
        $baselineFiles = @(Get-ChildItem -LiteralPath $resolvedBase -Filter '*.png' -File | Sort-Object Name)
        if ($baselineFiles.Count -eq 0) {
            Write-Log ("no baseline PNGs in {0}" -f $resolvedBase) 'error'
            $compareResults += [pscustomobject]@{
                scenario = '*'; status = 'no_baselines'; note = $resolvedBase
                deltaPercent = $null; diffPng = $null
            }
        }
        foreach ($bf in $baselineFiles) {
            $id = [System.IO.Path]::GetFileNameWithoutExtension($bf.Name)
            $current = Join-Path $resolvedOut $bf.Name
            if (-not (Test-Path -LiteralPath $current -PathType Leaf)) {
                $r = [pscustomobject]@{
                    scenario = $id; status = 'missing_current'
                    note = ("scenario {0} produced no valid frame this run, so no comparison was possible" -f $id)
                    deltaPercent = $null; diffPng = $null
                }
                $compareResults += $r
                Write-Log ("  {0}: MISSING CURRENT" -f $id) 'error'
                continue
            }
            $diffPath = Join-Path $compareDir ('diff-' + $bf.Name)
            $ignoreTop = 0
            $sr = $scenarioResults | Where-Object { $_.id -eq $id } | Select-Object -First 1
            if ($sr) { $ignoreTop = [int]$sr.ignoredTopRows }
            $cr = [ShImagesVisual.Frames]::Compare($bf.FullName, $current, $DeltaTolerance, $diffPath, $ignoreTop)
            $r = [pscustomobject]@{
                scenario        = $id
                status          = $cr.Status
                note            = $cr.Note
                deltaPercent    = [Math]::Round($cr.DeltaPercent, 4)
                differingPixels = $cr.DifferingPixels
                comparedPixels  = $cr.ComparedPixels
                totalPixels     = $cr.TotalPixels
                tolerance       = $cr.Tolerance
                ignoredTopRows  = $cr.IgnoredTopRows
                baselineSize    = ("{0}x{1}" -f $cr.BaselineWidth, $cr.BaselineHeight)
                currentSize     = ("{0}x{1}" -f $cr.CurrentWidth, $cr.CurrentHeight)
                diffPng         = (Split-Path -Leaf $diffPath)
            }
            $compareResults += $r
            Write-Log ("  {0}: {1} delta {2}%" -f $id, $cr.Status, $r.deltaPercent)
        }
        $haveBase = @($baselineFiles | ForEach-Object { $_.BaseName })
        foreach ($sr in $scenarioResults) {
            if ($sr.status -eq 'captured' -and ($haveBase -notcontains $sr.id)) {
                $compareResults += [pscustomobject]@{
                    scenario = $sr.id; status = 'no_baseline'
                    note = 'captured this run but no committed baseline exists yet; add it deliberately, not automatically'
                    deltaPercent = $null; diffPng = $null
                }
                Write-Log ("  {0}: NO BASELINE" -f $sr.id) 'warn'
            }
        }
    }
}

# --------------------------------------------------------------------------
# Summary + run-root cleanup.
# --------------------------------------------------------------------------
$captured = @($scenarioResults | Where-Object { $_.status -eq 'captured' })
$failedCount = @($scenarioResults | Where-Object { $_.status -ne 'captured' }).Count

$summary = [ordered]@{
    schemaVersion      = 1
    generatedUtc       = (Get-Date).ToUniversalTime().ToString('o')
    tool               = 'tools/visual/capture.ps1'
    advisoryOnly       = $true
    advisoryNote       = 'Human-reviewed artifact. Pixel output varies with GPU driver, Windows build, DPI scale and font fallback, so this tool is NOT a gate and never fails on a pixel delta. Geometry invariants in crates/sh-app/src/app.rs are the gating mechanism. See docs/ARCHITECTURE.md ADR-022.'
    exePath            = $exeFull
    dpiAware           = $dpiAware
    captureMechanism   = 'PrintWindow(hwnd, hwnd, PW_RENDERFULLCONTENT=2)'
    rejectedMechanism  = 'CopyFromScreen: loses z-order fights and captured the wrong window'
    pinnedInput        = 'mouse pointer parked at (0,0) before every scenario and restored on exit. Without this a hovered grid cell paints an elevated card 180px wide against the 160px of every other cell, which reads as a layout regression and is not one.'
    excludedBand     = 'rows above the client area (GetClientRect/ClientToScreen) are excluded from every pixel delta: the native title bar is drawn by Windows, paints a gradient when focused and flat #202020 when not, and cannot be pinned because the foreground lock refuses a background process.'
    windowTimeoutSec   = $WindowTimeoutSec
    maxStabilityAttempts = $MaxStabilityAttempts
    blankDominantShareThreshold = $BlankDominantShare
    deltaTolerancePerChannel   = $DeltaTolerance
    hermeticProfileDir = $profileDir
    fixtureDir         = $fixtureDir
    fixtures           = $fixtureDigests
    scenarios          = $scenarioResults
    compareEnabled     = [bool]$Compare
    compareBaselineDir = $(if ($Compare) { [System.IO.Path]::GetFullPath($BaselineDir) } else { $null })
    compareResults     = $compareResults
    counts             = [ordered]@{
        scenariosTotal    = $scenarioResults.Count
        scenariosCaptured = $captured.Count
        scenariosFailed   = $failedCount
        pngsWritten       = $captured.Count
    }
    fatalError         = $fatal
}

$summaryPath = Join-Path $resolvedOut 'summary.json'
$json = $summary | ConvertTo-Json -Depth 8
# PowerShell 5.1's Set-Content/Out-File write a UTF-8 BOM; write through .NET
# so the file is clean UTF-8 for jq and for the workflow's ConvertFrom-Json.
[System.IO.File]::WriteAllText($summaryPath, $json, (New-Object System.Text.UTF8Encoding($false)))

Write-Log ("captured {0}/{1} scenarios, {2} failed" -f $captured.Count, $scenarioResults.Count, $failedCount)
Write-Log ("summary: {0}" -f $summaryPath)
Write-Log ("PNG_COUNT={0}" -f $captured.Count)
if ($captured.Count -gt 0) {
    Write-Log 'These frames are for human review only. A non-zero delta below is not a build failure.'
}
if ($Compare) {
    Write-Log 'Compare mode reported deltas above. It did not, and must not, fail on them.'
}

# Keep the run root only when something went wrong: a successful run's temp
# files are pure litter, and a failed run's are the reproduction recipe.
if ($captured.Count -eq $scenarioResults.Count -and -not $KeepRunRoot -and $null -eq $fatal) {
    try {
        Remove-Item -LiteralPath $runRoot -Recurse -Force
    } catch {
        Write-Log ("could not remove run root {0}: {1}" -f $runRoot, $_.Exception.Message) 'warn'
    }
} else {
    Write-Log ("run root kept for reproduction: {0}" -f $runRoot)
}

if ($captured.Count -eq 0) {
    Write-Log 'no valid frame was captured for any scenario' 'error'
    exit 1
}
exit 0