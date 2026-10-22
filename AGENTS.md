# AGENTS.md

Bad pixel analyzer for ASTM E2597-22: opens a detector image (TIFF, PNG, DICONDE), classifies the defects per the standard and writes a text report. Single-crate egui/eframe app, native (Windows) + wasm.
All app logic lives in `src/main.rs`; `index.html` is the trunk host page; the icons are in `assets/icons`.

## Status

Implemented: `src/main.rs` realizes the spec below for both native and wasm targets. `cargo test` covers the classification rules, the detector rim, the preview colours and the status bar readout (16 unit tests in `src/main.rs`).

## Input images

The user can open TIFF or PNG files, both 16 and 8 bit, as well as DICONDE (DICOM). In the opened image the pixels with zero intensity are considered "good" and all other pixels are considered bad.

- TIFF/PNG go through the `image` crate (`to_luma16`); DICONDE through `dicom-object` + `dicom-pixeldata` (`decode_pixel_data`).
- DICOM is read from the **raw stored values** (no rescale slope/intercept, no VOI LUT), so a zero stays a zero. Only the first frame is used. The decoder is picked by the file extension (`.dcm`, `.diconde`) or by the `DICM` preamble at offset 128.
- The `image` crate has no DICOM support — that is why `dicom-object`/`dicom-pixeldata` are needed.

## Classification rules

* Single bad pixel - A single pixel with only good (8) neighborhood pixels.
* Cluster of Bad Pixels — Two or more connected bad pixels (8-connectivity).
* Cluster Kernel Pixels (CKP) — the pixels which do not have five or more good neighborhood pixels.
* A pixel at the detector rim edge (physical edge or defined border) is not called CKP if it has three or more good neighbors.
* A pixel in a detector rim corner with less than 2 good neighbors is called a CKP.
* A cluster without any CKP is labeled as irrelevant cluster.
* A cluster with CKP is labeled a relevant cluster.
* A bad line segment is a special cluster with 10 or more pixels connected in a line (row or column), where no more than 10% of this line has adjacent bad pixels.
* If a relevant cluster is located at the end of a bad line segment, then bad line segment is separated from the relevant cluster.

Implementation notes (`analyze` in `src/main.rs`):
- The image border is the detector rim; only the in-image neighbours are counted as good.
- A line run is trimmed at both ends while the end pixel has a bad pixel **across** the line direction — that is how a cluster at the end of a line is separated from the line — and while the end pixel is a **kernel pixel that touches that cluster** (see below). The trimmed run must still be ≥ 10 pixels and have ≤ 10% pixels with an adjacent bad pixel.
- Bad pixels connected to the **side** of a line belong to the line segment (they are counted in its name and in its bounding rectangle) — unless the group they form contains a kernel pixel, see below.
- A component can hold **more than one line**: a horizontal and a vertical line that cross are two line segments, each with its own name. A line is **not** split by a crossing or by a cluster that sits on it — the line stays one segment and is marked with a **"C"** at the end of its name (e.g. `1x126 Line126C`).
- The crossings of the lines are **not** reported as clusters: the crossing pixels belong to the lines. A crossing only becomes a defect when there are extra kernel pixels next to it, which are then reported as a cluster that sits on the line.
- A bad group next to the **side** of a line that contains a **kernel pixel** is a cluster of its own: it is separated from the line and reported (e.g. `rel2x2 cluster4-2`). A side group without a kernel pixel is a part of the line and is counted in its name (e.g. `2x24 Line25`).
- A **kernel pixel of a line** that touches a cluster (8-connected) belongs to that cluster: it is a kernel pixel of the cluster as well. Such a pixel sits at the end of the line, where the cluster is attached to it, so it is **cut from the line** (the line is one pixel shorter) and the cluster grows over the end of the line — e.g. a 2×3 block at the end of a line gives `1x24 Line24 + rel3x3 cluster7-7`: the block, the two line pixels under it and the last pixel of the line. The pixel is counted once, in the cluster.
- The CKP flags are computed on the original image (the line pixels still count as bad neighbours of the cluster next to them).

## Naming conventions

* The name of irrelevant cluster is the size of the rectangle around the cluster and number of bad pixels in the irrelevant cluster, for example "2x3 cluster4".
* The name of relevant cluster is the size of the rectangle around the cluster and number of bad pixels in the relevant cluster with prefix "rel" and the number of CKPs as a suffix, for example "rel3x4 cluster7-2".
* Bad Line Segment named as "1x24 Line24". If any bad pixel connected to side of the line then the name is rectanle around the line and total amound of bad pixels including connected, for example, "2x24 Line26". A line that is crossed is marked with a "C" at the end, for example "1x24 Line24C".

The rectangle is named as **rows × columns**, so a horizontal line of 24 pixels is "1x24 Line24" and a vertical one is "24x1 Line24".

## Generated Report

The report is written by **Generate Report** (Ctrl+G): a native save dialog on desktop, a browser download on web. It is also copied to the clipboard by **Copy Report to Clipboard** (Ctrl+Shift+C) — the same text.

The format is the ASTM summary followed by the ASTM 2737 report for the relevant clusters:

```
ASTM Summary
Total Defects = 170
Defects Area, % = 16,601562
Single Bad Pixels = 1
Bad Line Segments = 5
Irrelevant Clusters = 2
Relevant Clusters = 4
Min Distance Between Adjacent Lines = 5
Max Density in 11x11 area = 29 Pixels
Non-Line Bad Pixels = 26
Cumulative Pixels in underperforming Lines = 139
single bad pixels (1)

bad line segments (5):
1x32 Line32	(Col 0, Row 2)
2x24 Line25	(Col 5, Row 12)
1x25 Line25	(Col 5, Row 19)
1x25 Line25	(Col 4, Row 24)
32x1 Line32	(Col 2, Row 0)

irrelevant clusters (2):
2x2 cluster2	(Col 12, Row 6)
2x3 cluster4	(Col 18, Row 6)

relevant clusters (4):
rel3x3 cluster5-5	(Col 1, Row 1)
rel3x4 cluster7-2	(Col 6, Row 25)
rel3x2 cluster6-6	(Col 30, Row 18)
rel3x2 cluster6-4	(Col 29, Row 23)

ASTM 2737 Report for Relevant Clusters

relevant Cluster #1	3x3 cluster5-5 No of ClusterKernelPixel = 	5
Position of relevant Cluster #1 Col1	1	Col2	3	Row1	1	Row2	3

relevant Cluster #2	3x4 cluster7-2 No of ClusterKernelPixel = 	2
Position of relevant Cluster #2 Col1	25	Col2	28	Row1	6	Row2	8
```

- All the coordinates of the report are given as **Col, Row** (column first, then row). The position of a defect is the top left corner of its rectangle; the ASTM 2737 part gives the bounds as `Col1 … Col2 … Row1 … Row2`. The list of the relevant clusters in the Report panel uses the same labels for the kernel pixel positions.

- **Total Defects** is the number of the defective pixels (all the defects together), not the number of the defects. Every bad pixel of the active area is counted **once**, also when it is a part of two defects (the crossing of two lines, for example) — the cross of a full row and a full column has 63 defective pixels, not 64.
- **Defects Area, %** is the share of the defective pixels of the *active area* (the image without the detector rim), with a comma as the decimal separator and six decimals.
- **Min Distance Between Adjacent Lines** is the shortest distance between the pixels of two **parallel** line segments (the perpendicular distance for lines that overlap along their length). The lines that are not parallel are not compared; the value is `n/a` when there is no parallel pair.
- **Max Density in 11x11 area** is the largest number of bad pixels found in an 11×11 window (`DENSITY_AREA`) sliding over the active area. The window is centred on every pixel of the active area and clipped at the image border.
- **Non-Line Bad Pixels** are the bad pixels that do not belong to a line segment.
- **Cumulative Pixels in underperforming Lines** are the pixels of all the line segments (the sum of their pixel counts, so a pixel where two lines cross is counted twice).
- **Bad Pixel Statistics** (at the end of the report) count the bad pixels of the active area (the line pixels included) by the number of bad pixels in their 3×3 box, the pixel itself included: **Box 1** are the isolated bad pixels (the single pixels), **Box 2** the bad pixels with one bad neighbour, … **Box 9** the bad pixels with all their neighbours bad. The good pixels are not counted, and the pixels outside of the active area are not bad. The sum of the boxes is the number of the bad pixels of the active area.
- The single bad pixels are only counted, not listed.
- In the ASTM 2737 part the name of a relevant cluster has no `rel` prefix.

## GUI

```
+------------------------------------------------------------------+
| Title bar                                                        |
+------------------------------------------------------------------+
| File Help                                                        |
+-----------------+--------------------+---------------------------+
|  = Tools =      |                    |  = Report =               |
| Zoom In         |                    | Single Bad Pixels         |
| Zoom Out        |                    | Irrelevant Clusters       |
| Fit to Screen   |                    | Line Segments             |
|                 |       IMAGE        | Relevant Clusters         |
|                 |                    | List of Relevant Clusters |
|                 |                    |                           |
|                 |                    |                           |
|                 |                    +---------------------------+
|                 |                    | [Generate Report]         |
|                 |                    |                           |
+-----------------+--------------------+---------------------------+
```

Interface is panels: menu bar on top, then three panels in the middle (Tools | Image | Report), plus a status bar at the bottom.

- The web version shows a **notice** under the title: the tool is experimental, for test purposes only, and it is a client-side app — the files are processed locally in the browser and are never uploaded to any server. The notice is in the web-only title bar panel, so the desktop build does not show it.
- The image area supports wheel zoom (anchored at the mouse pointer — `zoom_step` keeps the anchor's texel fraction fixed) and drag-to-pan; zoom = 1.0 is fit-to-screen. The pan is limited (`limit_pan`), so a part of the image (`VISIBLE_MARGIN` = 8 points) always stays visible — the image can not be dragged out of the view.
- The cursor over the image is the default pointer, except when the image is being dragged (panned), when it shows the grabbing hand. It returns to the default pointer as soon as the mouse button is released.
- Tools panel holds six 36×36 buttons, no caption: zoom_in, zoom_out, fit_to_screen (20×20 icons), then **1:1** (a text button — the actual size, one image pixel is one point of the screen; it uses the fit scale of the last frame) and then the navigation between the relevant clusters — prev_cluster (arrow_left) and next_cluster (arrow_right, 24×24 icons). The panel is `exact_size(52.0)` so the gaps left/right of each button match.
- **Prev/Next cluster** show the relevant clusters one by one (the order of the report), wrapping around. The cluster is put in the middle of the view and the view is zoomed in, when the cluster is too small to be seen — it is never zoomed out. The status bar shows `Relevant cluster #N of M`. The buttons are disabled when there is no relevant cluster.
- The **left and right arrow keys** navigate over the relevant clusters, like the Prev/Next buttons do (the tooltips of the buttons mention the keys). The keys are handled in `show_menu`, so they work everywhere in the app, and they are left alone when the About or the report window is open.
- The cluster that is shown gets a **caption above it** (`ClusterCaption`): `#N of M  <name>`, on a dark translucent background. The caption disappears as soon as the image is panned or zoomed (drag, wheel, zoom buttons, scrollbars, thumbnail click, Fit to Screen).
- Report panel has the 4 indicators plus a scrollable list of relevant clusters. The indicators live in a 2-column `egui::Grid` (id "report") so labels stay left-aligned and all numeric boxes start on one vertical line; a row of three buttons of the same width - **Save** (the report in a file, Ctrl+G), **Copy** (the report in the clipboard, Ctrl+Shift+C) and **View** (the report in a window) - sits under the scroll bar. The panel is resizable but never narrower than 205 points, so that the indicators and buttons always fit. The thumbnail and the **Detector Rim** control are above the indicators.
- Between the indicators and the relevant clusters there is a **scrollable text area with the report** (`App::report`, a monospace label that wraps the long lines). It shows exactly the text that **Generate Report** saves and **Copy Report to Clipboard** copies, so the report can be read without saving it. The text is rebuilt whenever the analysis changes (`set_image`, `apply_rim`).
- File menu: **Open Image...** (Ctrl+O), **Generate Report** (Ctrl+G), **Copy Report to Clipboard** (Ctrl+Shift+C), **Quit** (Ctrl+Q, native only). View menu: **View Report...** (no shortcut). Help menu: About.
- **View → View Report...** opens a modal window with the whole report in monospace, about the size of the app (90 % of the viewport width, 85 % of its height), with a scroll bar and a Close button. The text is the same as the one that is saved and copied.
- The image can also be opened by **dragging a file onto the window** (the file dialog stays available). egui delivers the drop as `RawInput::dropped_files`; `App::load_dropped` reads the bytes (`DroppedFile::bytes` on the desktop, `bytes_async` in the browser) and opens the image. The web version reads the dropped file in the background and opens it in a later frame.
- The **desktop version** remembers the path of the last opened image in the registry of the user: `HKCU\Software\ASTM`, the string value `last_badpix_image` (the module `last_image` in `src/main.rs`, the crate `winreg`). The key is created when it does not exist yet, and only the absolute paths are stored. The **Open Image** dialog starts in the directory of that image. The web version has no registry and keeps no such setting.
- Shortcuts (constants near the top of `main.rs`): Generate Report = Ctrl+G, Open Image = Ctrl+O, Copy Report = Ctrl+Shift+C, Quit = Ctrl+Q (native only), Previous/Next relevant cluster = Left/Right arrow.

### Detector rim

Above the Report panel there is one integer control, **Detector Rim** (default 0, never negative, at most half the image minus one pixel). It is the width of the border that is cut from the active area of the detector:

- The rim pixels are **not evaluated**: they are neither good nor bad, they are not counted and they do not connect the clusters on their sides.
- The rim is the **edge of the detector**: the pixels of the active area next to it are the rim edge pixels of the standard (three good neighbours) and the pixels in its corners are the rim corner pixels (two good neighbours). The good neighbours are counted inside the active area only.
- The rim is drawn **gray** in the preview, so the active area is visible at a glance. The bad pixels of the rim are drawn **dark gray**: they are shown, but they are not evaluated (`Analysis::rim_bad`).
- Changing the rim re-runs the analysis and rebuilds the texture (`App::apply_rim`); the report gets a `Detector rim: N px (active area W x H)` line.

### Status bar

From left to right: the opened image and its size, the position in **X Y** form, the kind of the pixel at that position, the zoom ratio and the last action message. When a relevant cluster is shown (via the arrow keys, the Prev/Next buttons or the thumbnail), the position is the top-left corner of the cluster's rectangle; otherwise it is the position of the mouse pointer.

- The pixel kind is **D** for a bad pixel, **K** for a cluster kernel pixel (red) and **C** for a good pixel that has a bad neighbour; a good pixel without bad neighbours shows nothing. The letters are the only place where `K`/`D`/`C` are used — do not rename them without a reason.
- The zoom ratio is the app's zoom factor (1.00 = fit to screen) followed by the scale of the display in screen pixels per image pixel, e.g. `Zoom: 2.50x (3.2 px/px)`.
- The mouse position is computed in the image view (which runs after the status bar), so the readout lags one frame behind the pointer.

### Image display

- The preview is the defect map, not the source image: good pixels are **black**, bad pixels are **white**, the cluster kernel pixels are **red** (a CKP overrides the white of its pixel). The colour of a pixel is picked by `pixel_color`.
- The texture is loaded with `egui::TextureOptions::NEAREST`, so the pixels stay sharp when zoomed in — interpolation is off. Do not switch to the default (linear) options.
- When one image pixel takes **64×64 screen points** or more (`PIXEL_DETAIL_SCALE`), a gray grid is drawn between the pixels (Photoshop style) and the letter of every visible pixel is drawn in its middle: **D** (bad pixel) in black, **K** (cluster kernel pixel) in yellow, **C** (good pixel next to a bad one) in white. The letters contrast with the pixel they are drawn on. Only the visible pixels are considered, so the cost stays low.
- The image view has **scrollbars** (`SCROLLBAR` = 12 points): they appear when the image does not fit into the view and their thumb is dragged to pan. The wheel still zooms, the drag still pans.
- The image is clipped to the view area, so it never draws under the scrollbars. The grid, the letters and the cluster caption are clipped to the same area, and the **scrollbars are painted last**, so they are always above them.
- The pan is clamped (`limit_pan`) so a part of the image always stays visible (`VISIBLE_MARGIN` = 8 points). The clamp bounds are ordered before clamping, so the pan cannot panic when the image fits entirely in the view and the lower bound is larger than the upper bound.
- The **Report panel is resizable** (it has a splitter handle). Dragging it can shrink the image area to zero width; in that case the image view stops drawing (no zero-size texture is sent to the renderer) and the image reappears as soon as the splitter is moved back. The thumbnail and the report buttons guard against non-positive dimensions the same way.

### Thumbnail

Above the **Detector Rim** control there is a small thumbnail of the image (at most 240 points wide and **200 points high**, so a tall image fits) with a red rectangle that marks the part of the image that is visible now. The thumbnail is **centered in the panel**, so that the narrow thumbnail of a tall image (e.g. 1000×4000) does not stick to the left edge, and it never sticks out of the panel. Clicking or dragging in the thumbnail moves the view to that part of the image. The thumbnail uses `App::display_rect` and `App::view_rect`, which the image view writes every frame — the thumbnail therefore lags one frame behind.

The preview of the image is split into **tiles** of 2048×2048 pixels (`TILE`), because egui refuses a texture with a side longer than 2048 — without the tiles a large image (e.g. 980×4308) would crash the app.

## Build & run

- Desktop (Windows MSVC): `cargo run`.
- Web: `cargo build --target wasm32-unknown-unknown` (target installed via rustup). The web entry panics unless `index.html` contains `<canvas id="the_canvas_id">` and `<div id="loading_text">` (the bootstrap removes the latter on startup).
- Web build check: `trunk build --release --public-url . --no-default-features` (writes `dist/`, which is git-ignored).
- Effective verification is `cargo build` on **both** targets, `cargo test` and `cargo fmt`. No clippy config.
- `rustfmt.toml` is non-default: **hard tabs** (not 4 spaces), `max_width = 100`, `use_small_heuristics = "Max"`. Run `cargo fmt` instead of hand-formatting.

### Test images

`assets/images` holds 14 detector images whose **names hold the expected classification**: the single bad pixels, the line segments, the irrelevant clusters and the relevant clusters (e.g. `1x24 line24 + rel3x3 cluster7-7.tif` is a 24 pixel line with a relevant cluster of 7 kernel pixels in a 3×3 rectangle). The test `the_test_images_are_classified_as_their_names_say` analyzes every one of them and compares the names of the defects with the expectations in the table of the test. When the classification rules change, this test shows immediately which images are affected.

### Web specifics

- eframe 0.36 calls `App::ui(&mut self, ui, frame)` — not `App::update`.
- The file is picked with a hidden `<input type="file">` created once and reused; the file is read in the background with `FileReader` (web-sys's `File` has no `array_buffer`), and the result is handed to the app through `Rc<RefCell<Option<...>>>`.
- `web-sys` features used here: Blob, Document, Event, EventTarget, File, FileList, FileReader, HtmlCanvasElement, HtmlElement, HtmlInputElement, Url, Window.

## Crates

eframe = "0.36.2" (eframe, egui and egui_extras must share the version)
egui = "0.36.2"
egui_extras = { version = "0.36.2", features = ["svg"] }
image = { version = "0.25.10", features = ["tiff"] }
dicom-core = "0.10.0"
dicom-object = "0.10.0"
dicom-pixeldata = "0.10.0"
rfd = "0.17.2"
winreg = "0.56" (desktop only: the registry of the last opened image)
js-sys = "0.3.103"
wasm-bindgen-futures = "0.4.76"
web-sys = "0.3.103"
getrandom = { version = "0.3", features = ["wasm_js"] } (wasm target only)
