// Bad pixel analyzer for ASTM E2597-22.
//
// Opens a detector image (TIFF, PNG or DICONDE), classifies the defects per the
// standard and generates a text report.
//
// All app logic lives in this file.

use eframe::egui;

const GENERATE_SHORTCUT: egui::KeyboardShortcut =
	egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::G);

const OPEN_SHORTCUT: egui::KeyboardShortcut =
	egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);

// Ctrl+Shift+C, so that it does not shadow the copy of the text fields.
const COPY_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(
	egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
	egui::Key::C,
);

#[cfg(not(target_arch = "wasm32"))]
const QUIT_SHORTCUT: egui::KeyboardShortcut =
	egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Q);

// The arrow keys navigate over the relevant clusters, like the buttons of the
// Tools panel do.
const PREV_CLUSTER_SHORTCUT: egui::KeyboardShortcut =
	egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::ArrowLeft);

const NEXT_CLUSTER_SHORTCUT: egui::KeyboardShortcut =
	egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::ArrowRight);

const ZOOM_IN_ICON: egui::ImageSource = egui::include_image!("../assets/icons/zoom_in.svg");

const ZOOM_OUT_ICON: egui::ImageSource = egui::include_image!("../assets/icons/zoom_out.svg");

const FIT_TO_SCREEN_ICON: egui::ImageSource =
	egui::include_image!("../assets/icons/fit_to_screen.svg");

const PREV_CLUSTER_ICON: egui::ImageSource = egui::include_image!("../assets/icons/arrow_left.svg");

const NEXT_CLUSTER_ICON: egui::ImageSource =
	egui::include_image!("../assets/icons/arrow_right.svg");

/// Minimal length of a bad line segment, pixels.
const LINE_MIN_PIXELS: usize = 10;

/// Maximum share of the line pixels allowed to have an adjacent bad pixel, percent.
const LINE_MAX_ADJACENT_PERCENT: usize = 10;

/// The size of one image pixel on the screen, points, from which the grid
/// between the pixels and the letter of every pixel are shown.
const PIXEL_DETAIL_SCALE: f32 = 64.0;

/// The width of a scrollbar of the image view, points.
const SCROLLBAR: f32 = 12.0;

/// The margin around a relevant cluster when it is shown, image pixels.
const CLUSTER_MARGIN: f32 = 8.0;

/// The part of the image in one texture. The image is split into tiles, so that a
/// large image fits into the maximum texture size of egui.
const TILE: usize = 2048;

/// The size of the area in which the density of the bad pixels is measured.
const DENSITY_AREA: usize = 11;

/// The part of the image that stays visible when it is panned, points.
const VISIBLE_MARGIN: f32 = 8.0;

#[derive(Clone, Copy)]
enum MenuCommand {
	OpenImage,
	GenerateReport,
	CopyReport,
	ViewReport,
	#[cfg(not(target_arch = "wasm32"))]
	Quit,
}

fn menu_item(
	ui: &mut egui::Ui,
	label: &str,
	shortcut: Option<egui::KeyboardShortcut>,
	enabled: bool,
) -> egui::Response {
	match shortcut {
		Some(shortcut) => ui.add_enabled(
			enabled,
			egui::Button::new(label).shortcut_text(ui.ctx().format_shortcut(&shortcut)),
		),
		None => ui.add_enabled(enabled, egui::Button::new(label)),
	}
}

// White, frameless fill for panels/CentralPanel (the app uses a fixed light look;
// egui's default panel frames would follow the context theme instead).
fn light_panel_frame() -> egui::Frame {
	egui::Frame::new()
		.fill(egui::Color32::WHITE)
		.inner_margin(egui::Margin::symmetric(8, 2))
		.stroke(egui::Stroke::NONE)
}

// 36×36 square icon button used in the Tools panel.
//
// The centered sub-layout is required: the Tools panel layout is left-aligned, so a
// plain `ui.add(...)` would pin the icon to the left inside the square button.
// `Button::new` (not `Button::image`) is used because `Button::image` caps icon
// images at the font height.
fn icon_button(
	ui: &mut egui::Ui,
	icon: egui::ImageSource,
	tooltip: &str,
	enabled: bool,
	icon_size: f32,
) -> egui::Response {
	ui.allocate_ui_with_layout(
		egui::Vec2::splat(36.0),
		egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
		|ui| {
			ui.add_enabled(
				enabled,
				egui::Button::new(
					egui::Image::new(icon).fit_to_exact_size(egui::Vec2::splat(icon_size)),
				)
				.min_size(egui::Vec2::splat(36.0)),
			)
			.on_hover_text(tooltip)
		},
	)
	.inner
}

// A grayscale detector image. Pixel value 0 means a good pixel, any other value
// means a bad pixel.
struct GrayImage {
	width: usize,
	height: usize,
	pixels: Vec<u16>,
}

// The caption of the relevant cluster that is shown above it.
struct ClusterCaption {
	// The number of the cluster, counting from one, and the total.
	index: usize,
	total: usize,
	// The name of the cluster.
	name: String,
	// The rectangle of the cluster, in image pixels.
	rect: (usize, usize, usize, usize),
}

// A defect found by the analysis: a cluster of bad pixels or a bad line segment.
struct Defect {
	// Name per the naming conventions, e.g. "rel3x4 cluster7-2".
	name: String,
	// Number of the bad pixels of the defect.
	pixel_count: usize,
	// The bounding rectangle of the defect: (min X, min Y, max X, max Y).
	rect: (usize, usize, usize, usize),
	// Cluster kernel pixels of the defect, with their X Y coordinates.
	ckp_positions: Vec<(usize, usize)>,
}

// The result of the analysis of an image.
struct Analysis {
	// Isolated bad pixels (a cluster of a single pixel).
	single_bad_pixels: usize,
	// Clusters without cluster kernel pixels.
	irrelevant_clusters: Vec<Defect>,
	// Straight runs of bad pixels.
	line_segments: Vec<Defect>,
	// Clusters with at least one cluster kernel pixel.
	relevant_clusters: Vec<Defect>,
	// Mask of the bad pixels.
	bad: Vec<bool>,
	// Mask of the cluster kernel pixels.
	ckp: Vec<bool>,
	// Mask of the detector rim: the border that is cut from the image.
	rim: Vec<bool>,
	// Mask of the bad pixels in the detector rim: they are shown, but not
	// evaluated.
	rim_bad: Vec<bool>,
	// The width of the detector rim, pixels.
	rim_width: usize,
	// The shortest distance between the adjacent parallel line segments.
	min_line_distance: Option<f32>,
	// The largest number of bad pixels in an area of DENSITY_AREA pixels.
	max_density: usize,
	// The number of the bad pixels that do not belong to a line segment.
	non_line_pixels: usize,
	// The number of the pixels of all the line segments.
	line_pixels: usize,
	// The bad pixels of the active area by the number of bad pixels in their
	// 3x3 box, the pixel itself included: Box 1 are the isolated bad pixels,
	// Box 9 the bad pixels with all their neighbours bad.
	boxes: [usize; 10],
}

// The active area of the detector: the image without the detector rim.
struct Area {
	// Left, top, right and bottom of the area. The right and the bottom are
	// exclusive.
	x0: usize,
	y0: usize,
	x1: usize,
	y1: usize,
}

impl Area {
	// The area without a rim of the given width. The area is empty when the rim
	// covers the whole image.
	fn new(width: usize, height: usize, rim: usize) -> Self {
		let rim = rim.min(width / 2).min(height / 2);

		Self { x0: rim, y0: rim, x1: width - rim, y1: height - rim }
	}

	// Checks whether the pixel is a part of the active area.
	fn contains(&self, x: i32, y: i32) -> bool {
		x >= self.x0 as i32
			&& y >= self.y0 as i32
			&& (x as usize) < self.x1
			&& (y as usize) < self.y1
	}
}

// Counts the good (zero intensity) neighbours of the pixel at (x, y). The pixels
// outside the active area do not count.
fn count_good_neighbors(image: &GrayImage, area: &Area, x: usize, y: usize) -> usize {
	let mut good = 0;

	for dy in -1i32..=1 {
		for dx in -1i32..=1 {
			if dx == 0 && dy == 0 {
				continue;
			}

			let nx = x as i32 + dx;

			let ny = y as i32 + dy;

			if !area.contains(nx, ny) {
				continue;
			}

			if image.pixels[ny as usize * image.width + nx as usize] == 0 {
				good += 1;
			}
		}
	}

	good
}

// Labels the connected components of the bad pixels (8-connectivity).
// Returns the components (the pixel indices of each) and the component index of
// every pixel.
fn label_components(bad: &[bool], width: usize, height: usize) -> (Vec<Vec<usize>>, Vec<usize>) {
	let count = bad.len();

	let mut component_of = vec![usize::MAX; count];

	let mut components: Vec<Vec<usize>> = Vec::new();

	let mut stack: Vec<usize> = Vec::new();

	for start in 0..count {
		if !bad[start] || component_of[start] != usize::MAX {
			continue;
		}

		let index = components.len();

		component_of[start] = index;

		stack.push(start);

		let mut component = Vec::new();

		while let Some(pixel) = stack.pop() {
			component.push(pixel);

			let x = pixel % width;

			let y = pixel / width;

			for dy in -1i32..=1 {
				for dx in -1i32..=1 {
					if dx == 0 && dy == 0 {
						continue;
					}

					let nx = x as i32 + dx;

					let ny = y as i32 + dy;

					if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height {
						continue;
					}

					let neighbor = ny as usize * width + nx as usize;

					if bad[neighbor] && component_of[neighbor] == usize::MAX {
						component_of[neighbor] = index;

						stack.push(neighbor);
					}
				}
			}
		}

		components.push(component);
	}

	(components, component_of)
}

// A straight run of bad pixels.
struct LineRun {
	// Pixel indices of the run, in order.
	pixels: Vec<usize>,
	// True for a run in a row, false for a run in a column.
	horizontal: bool,
}

// Checks that the run qualifies as a bad line segment.
//
// The run is trimmed first: the pixels at its ends that are connected to a bad
// pixel across the line direction belong to a cluster at the end of the line,
// not to the line itself. A kernel pixel that touches such a cluster belongs to
// it as well, even when it has no bad pixel across the line direction. The
// remaining run qualifies when it is long enough and when no more than 10% of
// its pixels have an adjacent bad pixel.
fn qualify_run(
	bad: &[bool],
	width: usize,
	height: usize,
	pixels: &[usize],
	horizontal: bool,
	ckp: &[bool],
	separated: &[bool],
) -> Option<LineRun> {
	// The neighbours across the line direction:
	let sides = if horizontal { [(0i32, -1i32), (0, 1)] } else { [(-1i32, 0i32), (1, 0)] };

	// The run pixels with an adjacent bad pixel:
	let adjacent: Vec<bool> = pixels
		.iter()
		.map(|&pixel| {
			let x = pixel % width;

			let y = pixel / width;

			sides.iter().any(|&(dx, dy)| {
				let nx = x as i32 + dx;

				let ny = y as i32 + dy;

				if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height {
					return false;
				}

				bad[ny as usize * width + nx as usize]
			})
		})
		.collect();

	// The run pixels that are kernel pixels touching a cluster: they belong to
	// the cluster, not to the line.
	let junction: Vec<bool> = pixels
		.iter()
		.map(|&pixel| {
			if !ckp[pixel] {
				return false;
			}

			let x = pixel % width;

			let y = pixel / width;

			let mut touches = false;

			for dy in -1i32..=1 {
				for dx in -1i32..=1 {
					if dx == 0 && dy == 0 {
						continue;
					}

					let nx = x as i32 + dx;

					let ny = y as i32 + dy;

					if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height {
						continue;
					}

					if separated[ny as usize * width + nx as usize] {
						touches = true;

						break;
					}
				}

				if touches {
					break;
				}
			}

			touches
		})
		.collect();

	let mut start = 0;

	let mut end = pixels.len();

	while start < end && (adjacent[start] || junction[start]) {
		start += 1;
	}

	while end > start && (adjacent[end - 1] || junction[end - 1]) {
		end -= 1;
	}

	let trimmed = &pixels[start..end];

	if trimmed.len() < LINE_MIN_PIXELS {
		return None;
	}

	let with_adjacent = adjacent[start..end].iter().filter(|&&flag| flag).count();

	if with_adjacent * 100 > trimmed.len() * LINE_MAX_ADJACENT_PERCENT {
		return None;
	}

	Some(LineRun { pixels: trimmed.to_vec(), horizontal })
}

// Finds the maximal straight runs of bad pixels. A run is split at the
// crossings of the lines later on.
fn find_line_runs(bad: &[bool], width: usize, height: usize) -> Vec<LineRun> {
	let mut runs = Vec::new();

	// Runs in the rows:
	for y in 0..height {
		let mut x = 0;

		while x < width {
			if !bad[y * width + x] {
				x += 1;

				continue;
			}

			let start = x;

			while x < width && bad[y * width + x] {
				x += 1;
			}

			let pixels: Vec<usize> = (start..x).map(|column| y * width + column).collect();

			runs.push(LineRun { pixels, horizontal: true });
		}
	}

	// Runs in the columns:
	for x in 0..width {
		let mut y = 0;

		while y < height {
			if !bad[y * width + x] {
				y += 1;

				continue;
			}

			let start = y;

			while y < height && bad[y * width + x] {
				y += 1;
			}

			let pixels: Vec<usize> = (start..y).map(|row| row * width + x).collect();

			runs.push(LineRun { pixels, horizontal: false });
		}
	}

	runs
}

// The bounding rectangle of the pixels: (min X, min Y, max X, max Y).
fn bounding_rect(pixels: &[usize], width: usize) -> (usize, usize, usize, usize) {
	let mut min_x = usize::MAX;

	let mut min_y = usize::MAX;

	let mut max_x = 0;

	let mut max_y = 0;

	for &pixel in pixels {
		let x = pixel % width;

		let y = pixel / width;

		min_x = min_x.min(x);

		min_y = min_y.min(y);

		max_x = max_x.max(x);

		max_y = max_y.max(y);
	}

	(min_x, min_y, max_x, max_y)
}

// The distance between the pixels of two line segments, in pixels.
fn line_distance(a: &[usize], b: &[usize], width: usize, horizontal: bool) -> f32 {
	let mut best = f32::MAX;

	for &pa in a {
		let (ax, ay) = (pa % width, pa / width);

		for &pb in b {
			let (bx, by) = (pb % width, pb / width);

			// The difference across the lines is a lower bound of the distance:
			let (across, along) = if horizontal {
				((ay as i32 - by as i32).abs() as f32, (ax as i32 - bx as i32).abs() as f32)
			} else {
				((ax as i32 - bx as i32).abs() as f32, (ay as i32 - by as i32).abs() as f32)
			};

			if across >= best {
				continue;
			}

			let distance = (across * across + along * along).sqrt();

			if distance < best {
				best = distance;
			}
		}
	}

	best
}

// The shortest distance between the parallel line segments. The lines that are
// not parallel are not compared.
fn min_parallel_distance(lines: &[(Vec<usize>, bool)], width: usize) -> Option<f32> {
	let mut best: Option<f32> = None;

	for (index, (pixels, horizontal)) in lines.iter().enumerate() {
		for (other_pixels, other_horizontal) in lines.iter().skip(index + 1) {
			if *horizontal != *other_horizontal {
				continue;
			}

			let distance = line_distance(pixels, other_pixels, width, *horizontal);

			best = Some(match best {
				Some(current) => current.min(distance),
				None => distance,
			});
		}
	}

	best
}

// The largest number of bad pixels in an area of the given size, the area
// sliding across the active area of the detector.
fn max_density(bad: &[bool], width: usize, height: usize, area: &Area, size: usize) -> usize {
	// The prefix sums of the bad pixels:
	let mut sums = vec![0u32; (width + 1) * (height + 1)];

	for y in 0..height {
		for x in 0..width {
			sums[(y + 1) * (width + 1) + x + 1] = sums[y * (width + 1) + x + 1]
				+ sums[(y + 1) * (width + 1) + x]
				- sums[y * (width + 1) + x]
				+ bad[y * width + x] as u32;
		}
	}

	let half = (size / 2) as i32;

	let mut maximum = 0;

	for y in area.y0..area.y1 {
		for x in area.x0..area.x1 {
			// The area around the pixel:
			let x0 = (x as i32 - half).max(0) as usize;

			let y0 = (y as i32 - half).max(0) as usize;

			let x1 = ((x as i32 + half + 1) as usize).min(width);

			let y1 = ((y as i32 + half + 1) as usize).min(height);

			let count = sums[y1 * (width + 1) + x1] + sums[y0 * (width + 1) + x0]
				- sums[y0 * (width + 1) + x1]
				- sums[y1 * (width + 1) + x0];

			maximum = maximum.max(count as usize);
		}
	}

	maximum
}

// The statistics of the bad pixels of the active area: Box N are the bad
// pixels with N bad pixels in their 3x3 box, the pixel itself included. The
// good pixels are not counted and the pixels outside of the active area are
// not bad.
fn bad_pixel_boxes(image: &GrayImage, area: &Area) -> [usize; 10] {
	let mut boxes = [0usize; 10];

	for y in area.y0..area.y1 {
		for x in area.x0..area.x1 {
			if image.pixels[y * image.width + x] == 0 {
				// The good pixels are not counted:
				continue;
			}

			// The bad pixels of the 3x3 box around the pixel:
			let mut bad = 0;

			for dy in -1i32..=1 {
				for dx in -1i32..=1 {
					let nx = x as i32 + dx;

					let ny = y as i32 + dy;

					if !area.contains(nx, ny) {
						continue;
					}

					if image.pixels[ny as usize * image.width + nx as usize] != 0 {
						bad += 1;
					}
				}
			}

			boxes[bad] += 1;
		}
	}

	boxes
}

// The neighbours across the line direction.
fn sides(horizontal: bool) -> [(i32, i32); 2] {
	if horizontal { [(0, -1), (0, 1)] } else { [(-1, 0), (1, 0)] }
}

// The neighbour of a pixel in the given direction.
fn neighbour(pixel: usize, dx: i32, dy: i32, width: usize, height: usize) -> Option<usize> {
	let x = (pixel % width) as i32 + dx;

	let y = (pixel / width) as i32 + dy;

	if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
		return None;
	}

	Some(y as usize * width + x as usize)
}

// The connected groups (8-connectivity) of the given pixels.
fn groups(pixels: &[usize], width: usize, height: usize) -> Vec<Vec<usize>> {
	let mut mask = vec![false; width * height];

	for &pixel in pixels {
		mask[pixel] = true;
	}

	let mut result = Vec::new();

	let mut stack: Vec<usize> = Vec::new();

	for &start in pixels {
		if !mask[start] {
			continue;
		}

		mask[start] = false;

		stack.push(start);

		let mut group = Vec::new();

		while let Some(pixel) = stack.pop() {
			group.push(pixel);

			let x = (pixel % width) as i32;

			let y = (pixel / width) as i32;

			for dy in -1i32..=1 {
				for dx in -1i32..=1 {
					if dx == 0 && dy == 0 {
						continue;
					}

					let nx = x + dx;

					let ny = y + dy;

					if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height {
						continue;
					}

					let neighbor = ny as usize * width + nx as usize;

					if mask[neighbor] {
						mask[neighbor] = false;

						stack.push(neighbor);
					}
				}
			}
		}

		result.push(group);
	}

	result
}

// Classifies the bad pixels of the image per ASTM E2597-22.
//
// The detector rim of the given width is cut from the image: its pixels are not
// evaluated and the rim is the edge of the detector, so the pixels next to it
// are the rim edge pixels of the standard.
fn analyze(image: &GrayImage, rim: usize) -> Analysis {
	let (width, height) = (image.width, image.height);

	let count = width * height;

	let area = Area::new(width, height, rim);

	// The rim is not a part of the active area:
	let rim_mask: Vec<bool> = (0..count)
		.map(|index| !area.contains((index % width) as i32, (index / width) as i32))
		.collect();

	// Pixels with zero intensity are good, all the others are bad:
	let value_bad: Vec<bool> = image.pixels.iter().map(|&value| value != 0).collect();

	let bad: Vec<bool> = (0..count).map(|index| !rim_mask[index] && value_bad[index]).collect();

	// The bad pixels of the rim are shown, but they are not evaluated:
	let rim_bad: Vec<bool> = (0..count).map(|index| rim_mask[index] && value_bad[index]).collect();

	// Cluster kernel pixels: bad pixels with too few good neighbours.
	// An interior pixel needs five good neighbours, a pixel on the detector rim
	// edge needs three, a pixel in a detector rim corner needs two.
	let mut ckp = vec![false; count];

	for y in 0..height {
		for x in 0..width {
			let index = y * width + x;

			if !bad[index] {
				continue;
			}

			let good = count_good_neighbors(image, &area, x, y);

			let on_x_edge = x == area.x0 || x + 1 == area.x1;

			let on_y_edge = y == area.y0 || y + 1 == area.y1;

			let required = match (on_x_edge, on_y_edge) {
				(true, true) => 2,
				(true, false) | (false, true) => 3,
				(false, false) => 5,
			};

			ckp[index] = good < required;
		}
	}

	// The straight runs of bad pixels. A component may have more than one of
	// them: a horizontal and a vertical line that cross are two line segments.
	let runs = find_line_runs(&bad, width, height);

	// The pixels that belong to a run that is long enough to be a line:
	let mut in_line_run = vec![false; count];

	for run in &runs {
		if run.pixels.len() < LINE_MIN_PIXELS {
			continue;
		}

		for &pixel in &run.pixels {
			in_line_run[pixel] = true;
		}
	}

	// The crossings of the lines: the pixels where a horizontal and a vertical
	// line meet. A line with a crossing is marked with a "C".
	let mut crossings = vec![false; count];

	{
		let mut horizontal = vec![false; count];

		let mut vertical = vec![false; count];

		for run in &runs {
			// Only the runs that can be a line make a crossing:
			if run.pixels.len() < LINE_MIN_PIXELS {
				continue;
			}

			for &pixel in &run.pixels {
				if run.horizontal {
					horizontal[pixel] = true;
				} else {
					vertical[pixel] = true;
				}
			}
		}

		for index in 0..count {
			crossings[index] = horizontal[index] && vertical[index];
		}
	}

	// The bad pixels next to the lines that are not a part of a long run. The
	// groups with a kernel pixel are clusters of their own that sit on the
	// line, the other ones are a part of the line.
	let mut separated = vec![false; count];

	let mut covered = vec![false; count];

	let mut absorbed = vec![false; count];

	{
		// The side pixels of every run:
		let mut side: Vec<usize> = Vec::new();

		for run in &runs {
			for &pixel in &run.pixels {
				for (dx, dy) in sides(run.horizontal) {
					if let Some(neighbor) = neighbour(pixel, dx, dy, width, height) {
						if bad[neighbor] && !in_line_run[neighbor] {
							side.push(neighbor);
						}
					}
				}
			}
		}

		// The groups of the side pixels:
		for group in groups(&side, width, height) {
			if group.iter().any(|&pixel| ckp[pixel]) {
				// The group is a cluster of its own: it is separated from the
				// line, the line pixels it touches belong to it as well.
				for &pixel in &group {
					separated[pixel] = true;

					for (dx, dy) in [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
						if let Some(neighbor) = neighbour(pixel, dx, dy, width, height) {
							if in_line_run[neighbor] {
								covered[neighbor] = true;
							}
						}
					}
				}
			} else {
				// The group is a part of the line.
				for &pixel in &group {
					absorbed[pixel] = true;
				}
			}
		}
	}

	// Bad line segments: the line run plus the bad pixels connected to its side.
	// A cluster at the end of the line is not a part of the line segment.
	let mut line_mask = vec![false; count];

	// The bad pixels that are already a part of a line segment:
	let mut side_used = vec![false; count];

	let mut line_segments = Vec::new();

	// The pixels of every line segment and its direction:
	let mut line_pixel_lists: Vec<(Vec<usize>, bool)> = Vec::new();

	for run in &runs {
		// The run, trimmed at the ends:
		let Some(qualified) =
			qualify_run(&bad, width, height, &run.pixels, run.horizontal, &ckp, &separated)
		else {
			continue;
		};

		// The bad pixels connected to the side of the line that are a part of it:
		let mut pixels = qualified.pixels.clone();

		for &pixel in &qualified.pixels {
			for (dx, dy) in sides(run.horizontal) {
				if let Some(neighbor) = neighbour(pixel, dx, dy, width, height) {
					if bad[neighbor] && absorbed[neighbor] && !side_used[neighbor] {
						side_used[neighbor] = true;

						pixels.push(neighbor);
					}
				}
			}
		}

		for &pixel in &pixels {
			line_mask[pixel] = true;
		}

		let (min_x, min_y, max_x, max_y) = bounding_rect(&pixels, width);

		// The rectangle is named as rows × columns, so a horizontal line of 24
		// pixels is named "1x24 Line24". The lines that are crossed are marked
		// with a "C":
		let crossed = qualified.pixels.iter().any(|&pixel| crossings[pixel]);

		let name = format!(
			"{}x{} Line{}{}",
			max_y - min_y + 1,
			max_x - min_x + 1,
			pixels.len(),
			if crossed { "C" } else { "" }
		);

		line_segments.push(Defect {
			name,
			pixel_count: pixels.len(),
			rect: (min_x, min_y, max_x, max_y),
			ckp_positions: Vec::new(),
		});

		line_pixel_lists.push((pixels, run.horizontal));
	}

	// The clusters: the bad pixels that are not a part of a line, plus the
	// clusters that sit on a line. The crossings of the lines are not a part of
	// the cluster analysis: the crossing pixels are the lines themselves.
	let mut cluster_mask = vec![false; count];

	for index in 0..count {
		if bad[index] && (!line_mask[index] || separated[index] || covered[index]) {
			cluster_mask[index] = true;
		}
	}

	let rest = cluster_mask;

	let (rest_components, _) = label_components(&rest, width, height);

	let mut single_bad_pixels = 0;

	let mut irrelevant_clusters = Vec::new();

	let mut relevant_clusters = Vec::new();

	for component in &rest_components {
		let (min_x, min_y, max_x, max_y) = bounding_rect(component, width);

		let ckp_positions: Vec<(usize, usize)> = component
			.iter()
			.filter(|&&index| ckp[index])
			.map(|&index| (index % width, index / width))
			.collect();

		// A single bad pixel is a cluster of one pixel:
		if component.len() == 1 {
			single_bad_pixels += 1;

			continue;
		}

		// The rectangle is named as rows × columns:
		let rows = max_y - min_y + 1;

		let columns = max_x - min_x + 1;

		if ckp_positions.is_empty() {
			// A cluster without cluster kernel pixels is an irrelevant cluster:
			irrelevant_clusters.push(Defect {
				name: format!("{}x{} cluster{}", rows, columns, component.len()),
				pixel_count: component.len(),
				rect: (min_x, min_y, max_x, max_y),
				ckp_positions,
			});
		} else {
			// A cluster with cluster kernel pixels is a relevant cluster:
			relevant_clusters.push(Defect {
				name: format!(
					"rel{}x{} cluster{}-{}",
					rows,
					columns,
					component.len(),
					ckp_positions.len()
				),
				pixel_count: component.len(),
				rect: (min_x, min_y, max_x, max_y),
				ckp_positions,
			});
		}
	}

	// The bad pixels outside of the lines and the pixels of the lines:
	let non_line_pixels = (0..count).filter(|&index| bad[index] && !line_mask[index]).count();

	let line_pixels = line_segments.iter().map(|segment| segment.pixel_count).sum();

	let max_density = max_density(&bad, width, height, &area, DENSITY_AREA);

	let boxes = bad_pixel_boxes(image, &area);

	Analysis {
		single_bad_pixels,
		irrelevant_clusters,
		line_segments,
		relevant_clusters,
		bad,
		ckp,
		rim: rim_mask,
		rim_bad,
		rim_width: rim,
		min_line_distance: min_parallel_distance(&line_pixel_lists, width),
		max_density,
		non_line_pixels,
		line_pixels,
		boxes,
	}
}

// The color of a pixel in the preview: the detector rim is gray (dark gray for
// the bad pixels of the rim, that are shown but not evaluated), the good pixels
// are black, the bad pixels are white and the cluster kernel pixels are red.
fn pixel_color(analysis: &Analysis, index: usize) -> egui::Color32 {
	if analysis.rim[index] {
		return if analysis.rim_bad[index] {
			egui::Color32::from_gray(64)
		} else {
			egui::Color32::GRAY
		};
	}

	if analysis.ckp[index] {
		return egui::Color32::RED;
	}

	if analysis.bad[index] {
		return egui::Color32::WHITE;
	}

	egui::Color32::BLACK
}

// Builds the preview textures: the detector rim is gray, good pixels are black,
// bad pixels are white and the cluster kernel pixels are red. The image is split
// into tiles, so that a large image fits into the texture size limit of egui.
fn make_textures(
	ctx: &egui::Context,
	analysis: &Analysis,
	width: usize,
	height: usize,
) -> Vec<egui::TextureHandle> {
	let mut textures = Vec::new();

	for y0 in (0..height).step_by(TILE) {
		for x0 in (0..width).step_by(TILE) {
			let x1 = (x0 + TILE).min(width);

			let y1 = (y0 + TILE).min(height);

			let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);

			for y in y0..y1 {
				for x in x0..x1 {
					pixels.extend_from_slice(&pixel_color(analysis, y * width + x).to_array());
				}
			}

			let color_image = egui::ColorImage::from_rgba_unmultiplied([x1 - x0, y1 - y0], &pixels);

			// Nearest neighbour sampling: the pixels stay sharp when zoomed in.
			textures.push(ctx.load_texture(
				&format!("preview_{x0}_{y0}"),
				color_image,
				egui::TextureOptions::NEAREST,
			));
		}
	}

	textures
}

// Draws the image into the given rect. The image is drawn tile by tile.
fn draw_image(
	painter: &egui::Painter,
	textures: &[egui::TextureHandle],
	size: egui::Vec2,
	rect: egui::Rect,
) {
	let tiles_x = (size.x as usize).div_ceil(TILE);

	for (index, texture) in textures.iter().enumerate() {
		let tx = index % tiles_x;

		let ty = index / tiles_x;

		let x0 = tx as f32 * TILE as f32;

		let y0 = ty as f32 * TILE as f32;

		let x1 = (x0 + TILE as f32).min(size.x);

		let y1 = (y0 + TILE as f32).min(size.y);

		// The rect of the tile:
		let tile_rect = egui::Rect::from_min_max(
			egui::pos2(
				rect.min.x + x0 / size.x * rect.width(),
				rect.min.y + y0 / size.y * rect.height(),
			),
			egui::pos2(
				rect.min.x + x1 / size.x * rect.width(),
				rect.min.y + y1 / size.y * rect.height(),
			),
		);

		painter.image(
			texture.id(),
			tile_rect,
			egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
			egui::Color32::WHITE,
		);
	}
}

// The text report of the analysis.
//
// The format follows the ASTM summary and the ASTM 2737 report for the
// relevant clusters. Note that the position of a defect in the summary is given
// as (row, column), while the bounds of a relevant cluster are given as
// (columns, rows) — the same as in the reference reports.
fn report_text(image: &GrayImage, analysis: &Analysis) -> String {
	let mut text = String::new();

	// The total number of the defective pixels: every bad pixel of the active
	// area is counted once, even when it is a part of two defects (the
	// crossing of two lines, for example).
	let total_defects = analysis.bad.iter().filter(|&&bad| bad).count();

	// The share of the active area of the detector that is defective:
	let area = (image.width - 2 * analysis.rim_width) * (image.height - 2 * analysis.rim_width);

	let percent = if area > 0 { total_defects as f64 / area as f64 * 100.0 } else { 0.0 };

	text.push_str("ASTM Summary\n");

	text.push_str(&format!("Total Defects = {total_defects}\n"));

	text.push_str(&format!("Defects Area, % = {}\n", format!("{percent:.6}").replace('.', ",")));

	text.push_str(&format!("Single Bad Pixels = {}\n", analysis.single_bad_pixels));

	text.push_str(&format!("Bad Line Segments = {}\n", analysis.line_segments.len()));

	text.push_str(&format!("Irrelevant Clusters = {}\n", analysis.irrelevant_clusters.len()));

	text.push_str(&format!("Relevant Clusters = {}\n", analysis.relevant_clusters.len()));

	// The shortest distance between the adjacent parallel lines:
	let distance = match analysis.min_line_distance {
		Some(value) if (value - value.round()).abs() < 0.01 => format!("{}", value.round() as i64),
		Some(value) => format!("{value:.2}"),
		None => "n/a".to_owned(),
	};

	text.push_str(&format!("Min Distance Between Adjacent Lines = {distance}\n"));

	// The largest number of bad pixels in an area of the detector:
	text.push_str(&format!(
		"Max Density in {DENSITY_AREA}x{DENSITY_AREA} area = {} Pixels\n",
		analysis.max_density
	));

	// The bad pixels outside of the lines and the pixels of the lines:
	text.push_str(&format!("Non-Line Bad Pixels = {}\n", analysis.non_line_pixels));

	text.push_str(&format!(
		"Cumulative Pixels in underperforming Lines = {}\n",
		analysis.line_pixels
	));

	text.push_str(&format!("single bad pixels ({})\n", analysis.single_bad_pixels));

	text.push('\n');

	text.push_str(&format!("bad line segments ({}):\n", analysis.line_segments.len()));

	for segment in &analysis.line_segments {
		text.push_str(&format!(
			"{}\t(Col {}, Row {})\n",
			segment.name, segment.rect.0, segment.rect.1
		));
	}

	text.push('\n');

	text.push_str(&format!("irrelevant clusters ({}):\n", analysis.irrelevant_clusters.len()));

	for cluster in &analysis.irrelevant_clusters {
		text.push_str(&format!(
			"{}\t(Col {}, Row {})\n",
			cluster.name, cluster.rect.0, cluster.rect.1
		));
	}

	text.push('\n');

	text.push_str(&format!("relevant clusters ({}):\n", analysis.relevant_clusters.len()));

	for cluster in &analysis.relevant_clusters {
		text.push_str(&format!(
			"{}\t(Col {}, Row {})\n",
			cluster.name, cluster.rect.0, cluster.rect.1
		));
	}

	text.push('\n');

	text.push_str("ASTM 2737 Report for Relevant Clusters\n");

	for (index, cluster) in analysis.relevant_clusters.iter().enumerate() {
		let number = index + 1;

		// The name of a relevant cluster without the "rel" prefix:
		let name = cluster.name.strip_prefix("rel").unwrap_or(&cluster.name);

		text.push_str(&format!(
			"relevant Cluster #{number}\t{name} No of ClusterKernelPixel = \t{}\n",
			cluster.ckp_positions.len()
		));

		let (min_x, min_y, max_x, max_y) = cluster.rect;

		text.push_str(&format!(
			"Position of relevant Cluster #{number} Col1\t{min_x}\tCol2\t{max_x}\tRow1\t{min_y}\tRow2\t{max_y}\n"
		));

		text.push('\n');
	}

	// The statistics of the bad pixels of the active area:
	text.push_str("Bad Pixel Statistics\n");

	text.push_str(
		"(Box N = the bad pixels with N bad pixels in their 3x3 box, the pixel itself \
		 included)\n",
	);

	for (box_index, count) in analysis.boxes.iter().enumerate().skip(1) {
		text.push_str(&format!("Box {box_index} = {count}\n"));
	}

	text
}

// Checks whether the file is a DICOM file (by its extension or its preamble).
fn is_dicom(name: &str, bytes: &[u8]) -> bool {
	let name = name.to_ascii_lowercase();

	if name.ends_with(".dcm") || name.ends_with(".diconde") {
		return true;
	}

	bytes.len() > 132 && &bytes[128..132] == b"DICM"
}

// Loads a TIFF or PNG image.
fn load_raster(bytes: &[u8]) -> Result<GrayImage, String> {
	let image = image::load_from_memory(bytes).map_err(|error| format!("Image error: {error}"))?;

	let gray = image.to_luma16();

	let width = gray.width() as usize;

	let height = gray.height() as usize;

	Ok(GrayImage { width, height, pixels: gray.into_raw() })
}

// Loads a DICONDE (DICOM) image. Only the first frame is used.
fn load_dicom(bytes: &[u8]) -> Result<GrayImage, String> {
	use dicom_pixeldata::PixelDecoder;

	let object = dicom_object::from_reader(std::io::Cursor::new(bytes.to_vec()))
		.map_err(|error| format!("DICOM error: {error}"))?;

	let decoded =
		object.decode_pixel_data().map_err(|error| format!("DICOM pixel data error: {error}"))?;

	let width = decoded.columns() as usize;

	let height = decoded.rows() as usize;

	// The raw stored values are used as they are: the rescale and the VOI LUT
	// are not applied, so a zero stays a zero.
	let pixels: Vec<u16> = if decoded.bits_allocated() == 16 {
		decoded.frame_data_ow(0).map_err(|error| format!("DICOM pixel data error: {error}"))?
	} else {
		decoded
			.frame_data(0)
			.map_err(|error| format!("DICOM pixel data error: {error}"))?
			.iter()
			.map(|&value| value as u16)
			.collect()
	};

	if pixels.len() < width * height {
		return Err("DICOM pixel data is shorter than the image size".into());
	}

	Ok(GrayImage { width, height, pixels })
}

// Loads an image from the file contents, picking the decoder by the file name
// and the DICOM preamble.
fn load_image(name: &str, bytes: &[u8]) -> Result<GrayImage, String> {
	if is_dicom(name, bytes) { load_dicom(bytes) } else { load_raster(bytes) }
}

// Saves the report to a file (native version).
// Ok(None) means the dialog was canceled, Ok(Some(status)) is the success message.
#[cfg(not(target_arch = "wasm32"))]
fn save_report(text: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
	let Some(path) = rfd::FileDialog::new()
		.add_filter("Text", &["txt"])
		.set_file_name("bad_pixel_report.txt")
		.save_file()
	else {
		return Ok(None);
	};

	std::fs::write(&path, text)?;

	Ok(Some(format!("Saved {}", path.display())))
}

// Saves the report to a file (web version, browser download).
// Ok(None) means the dialog was canceled, Ok(Some(status)) is the success message.
#[cfg(target_arch = "wasm32")]
fn save_report(text: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
	use eframe::wasm_bindgen::JsCast as _;
	use web_sys::{Blob, Url};

	let array = js_sys::Uint8Array::from(text.as_bytes());

	let seq = js_sys::Array::new();

	seq.push(&array.buffer());

	let blob = Blob::new_with_u8_array_sequence(&seq).unwrap();

	let url = Url::create_object_url_with_blob(&blob).unwrap();

	let document = web_sys::window().unwrap().document().unwrap();

	let a = document.create_element("a").unwrap();

	a.set_attribute("href", &url).unwrap();

	a.set_attribute("download", "bad_pixel_report.txt").unwrap();

	let a: web_sys::HtmlElement = a.dyn_into().unwrap();

	a.click();

	Url::revoke_object_url(&url).unwrap();

	Ok(Some("Downloaded bad_pixel_report.txt".into()))
}

struct App {
	// The opened image and its analysis:
	image: Option<GrayImage>,
	analysis: Option<Analysis>,
	textures: Vec<egui::TextureHandle>,
	image_name: String,

	// View: zoom = 1.0 fits the image to the image area; pan offsets it from there.
	zoom: f32,
	pan: egui::Vec2,
	pending_zoom: Option<f32>, // factor requested by the Tools buttons, applied by the image view

	// The pixel under the mouse pointer and the scale of the display, both
	// updated by the image view:
	hover_pixel: Option<(usize, usize)>,
	pixel_scale: f32,

	// The rectangle of the image and of the view, both updated by the image
	// view. The thumbnail of the Report panel uses them.
	display_rect: Option<egui::Rect>,
	view_rect: Option<egui::Rect>,

	// The scale of the image at zoom = 1, updated by the image view. It is
	// needed for the actual size (1:1) of the image.
	fit_scale: f32,

	// The width of the detector rim: the border of the image that is cut from
	// the active area of the detector, pixels.
	detector_rim: usize,

	// The index of the relevant cluster that is shown now.
	cluster_index: Option<usize>,

	// The caption above the relevant cluster, if it is shown.
	cluster_caption: Option<ClusterCaption>,

	// The text of the report, shown in the Report panel.
	report: String,

	// Is the report window open?
	show_report_window: bool,

	// UI state:
	status: String,
	show_about: bool,

	// The files picked in the browser or dropped on the page, read in the
	// background:
	#[cfg(target_arch = "wasm32")]
	pending_files: std::rc::Rc<std::cell::RefCell<Vec<Result<(String, Vec<u8>), String>>>>,
	#[cfg(target_arch = "wasm32")]
	file_input: Option<web_sys::HtmlInputElement>,
	#[cfg(target_arch = "wasm32")]
	file_handler: Option<eframe::wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>>,
}

impl Default for App {
	fn default() -> Self {
		Self {
			image: None,
			analysis: None,
			textures: Vec::new(),
			image_name: String::new(),
			zoom: 1.0,
			pan: egui::Vec2::ZERO,
			pending_zoom: None,
			hover_pixel: None,
			pixel_scale: 1.0,
			display_rect: None,
			view_rect: None,
			fit_scale: 1.0,
			detector_rim: 0,
			cluster_index: None,
			cluster_caption: None,
			report: String::new(),
			show_report_window: false,
			status: String::new(),
			show_about: false,
			#[cfg(target_arch = "wasm32")]
			pending_files: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
			#[cfg(target_arch = "wasm32")]
			file_input: None,
			#[cfg(target_arch = "wasm32")]
			file_handler: None,
		}
	}
}

impl App {
	// Stores a newly opened image, analyzes it and builds the preview texture.
	fn set_image(&mut self, name: String, image: GrayImage, ctx: &egui::Context) {
		let analysis = analyze(&image, self.detector_rim);

		let textures = make_textures(ctx, &analysis, image.width, image.height);

		self.report = report_text(&image, &analysis);

		self.image = Some(image);

		self.analysis = Some(analysis);

		self.textures = textures;

		self.image_name = name;

		self.zoom = 1.0;

		self.pan = egui::Vec2::ZERO;

		self.cluster_index = None;

		self.cluster_caption = None;

		self.status = "Image analyzed".to_owned();
	}

	// Zooms the image view by `factor` around the fixed screen point `anchor`.
	// `fitted` is where the image is drawn at zoom = 1.
	fn zoom_step(&mut self, fitted: egui::Rect, anchor: egui::Pos2, factor: f32) {
		let display_min = fitted.min + self.pan;

		let display_size = fitted.size() * self.zoom;

		let zoom = (self.zoom * factor).clamp(0.05, 128.0);

		// Where the anchor sits on the image, as a fraction of the displayed area;
		// kept constant, so zooming follows the pointer:
		let u = ((anchor.x - display_min.x) / display_size.x).clamp(0.0, 1.0);

		let v = ((anchor.y - display_min.y) / display_size.y).clamp(0.0, 1.0);

		let new_size = fitted.size() * zoom;

		let new_min = anchor - egui::Vec2::new(u * new_size.x, v * new_size.y);

		self.zoom = zoom;

		self.pan = new_min - fitted.min;
	}

	// Shows the toolbar (menu bar) and handles the global keyboard shortcuts.
	// Returns None if no command was triggered this frame.
	fn show_menu(&mut self, ui: &mut egui::Ui) -> Option<MenuCommand> {
		let mut command = None;

		if ui.input_mut(|input| input.consume_shortcut(&GENERATE_SHORTCUT)) {
			command = Some(MenuCommand::GenerateReport);
		}

		if ui.input_mut(|input| input.consume_shortcut(&OPEN_SHORTCUT)) {
			command = Some(MenuCommand::OpenImage);
		}

		if ui.input_mut(|input| input.consume_shortcut(&COPY_SHORTCUT)) {
			command = Some(MenuCommand::CopyReport);
		}

		#[cfg(not(target_arch = "wasm32"))]
		if ui.input_mut(|input| input.consume_shortcut(&QUIT_SHORTCUT)) {
			command = Some(MenuCommand::Quit);
		}

		// The arrow keys navigate over the relevant clusters:
		self.navigate_clusters(ui.ctx());

		let has_image = self.image.is_some();

		let mut open_about = false;

		egui::MenuBar::new().ui(ui, |ui| {
			ui.menu_button("File", |ui| {
				if menu_item(ui, "Open Image...", Some(OPEN_SHORTCUT), true).clicked() {
					command = Some(MenuCommand::OpenImage);
				}

				ui.separator();

				if menu_item(ui, "Generate Report", Some(GENERATE_SHORTCUT), has_image).clicked() {
					command = Some(MenuCommand::GenerateReport);
				}

				if menu_item(ui, "Copy Report to Clipboard", Some(COPY_SHORTCUT), has_image)
					.clicked()
				{
					command = Some(MenuCommand::CopyReport);
				}

				ui.separator();

				#[cfg(not(target_arch = "wasm32"))]
				if menu_item(ui, "Quit", Some(QUIT_SHORTCUT), true).clicked() {
					command = Some(MenuCommand::Quit);
				}
			});

			ui.menu_button("View", |ui| {
				if menu_item(ui, "View Report...", None, has_image).clicked() {
					command = Some(MenuCommand::ViewReport);
				}
			});

			ui.menu_button("Help", |ui| {
				if ui.button("About...").clicked() {
					open_about = true;
				}
			});
		});

		if open_about {
			self.show_about = true;
		}

		command
	}

	// Limits the pan, so that a part of the image is always visible in the view.
	// The image may not be moved completely out of the view.
	fn limit_pan(&mut self, fitted: egui::Rect, area: egui::Rect) {
		// The size of the image on the screen:
		let size = fitted.size() * self.zoom;

		// The right edge of the image must be to the right of the view, and its
		// left edge must be to the left of the view. A margin of the image stays
		// visible at the limits:
		let min_x = area.min.x - fitted.min.x - size.x + VISIBLE_MARGIN;

		let max_x = area.max.x - fitted.min.x - VISIBLE_MARGIN;

		let min_y = area.min.y - fitted.min.y - size.y + VISIBLE_MARGIN;

		let max_y = area.max.y - fitted.min.y - VISIBLE_MARGIN;

		// When the image fits entirely in the view, min is larger than max and
		// clamp would panic. Keep the range valid by always ordering the bounds:
		self.pan.x = self.pan.x.clamp(min_x.min(max_x), min_x.max(max_x));

		self.pan.y = self.pan.y.clamp(min_y.min(max_y), min_y.max(max_y));
	}

	// Shows the Tools panel (left) — three 36×36 icon buttons for the zoom and two
	// for the navigation between the relevant clusters, no caption.
	fn show_tools(&mut self, ui: &mut egui::Ui) {
		ui.add_space(4.0);

		let has_image = self.image.is_some();

		if icon_button(ui, ZOOM_IN_ICON, "Zoom In", has_image, 20.0).clicked() {
			self.pending_zoom = Some(1.25);
		}

		if icon_button(ui, ZOOM_OUT_ICON, "Zoom Out", has_image, 20.0).clicked() {
			self.pending_zoom = Some(1.0 / 1.25);
		}

		if icon_button(ui, FIT_TO_SCREEN_ICON, "Fit to Screen", has_image, 20.0).clicked() {
			self.zoom = 1.0;

			self.pan = egui::Vec2::ZERO;

			self.cluster_caption = None;
		}

		// The actual size of the image: one pixel of the image is one point of
		// the screen.
		let actual_size =
			ui.add_sized([36.0, 36.0], egui::Button::new("1:1")).on_hover_text("Actual Size (1:1)");

		if actual_size.clicked() && has_image && self.fit_scale > 0.0 {
			self.pending_zoom = Some(1.0 / (self.fit_scale * self.zoom));
		}

		ui.add_space(6.0);

		let has_clusters = self
			.analysis
			.as_ref()
			.map(|analysis| !analysis.relevant_clusters.is_empty())
			.unwrap_or(false);

		if icon_button(
			ui,
			PREV_CLUSTER_ICON,
			"Previous Relevant Cluster (Left arrow)",
			has_clusters,
			24.0,
		)
		.clicked()
		{
			self.prev_cluster();
		}

		if icon_button(
			ui,
			NEXT_CLUSTER_ICON,
			"Next Relevant Cluster (Right arrow)",
			has_clusters,
			24.0,
		)
		.clicked()
		{
			self.next_cluster();
		}
	}

	// The left and right arrows navigate over the relevant clusters, like the
	// buttons of the Tools panel do. The keys are left alone when a window is
	// open on top of the app.
	fn navigate_clusters(&mut self, ctx: &egui::Context) {
		if self.show_about || self.show_report_window {
			return;
		}

		let previous = ctx.input_mut(|input| input.consume_shortcut(&PREV_CLUSTER_SHORTCUT));

		let next = ctx.input_mut(|input| input.consume_shortcut(&NEXT_CLUSTER_SHORTCUT));

		if previous {
			self.prev_cluster();
		}

		if next {
			self.next_cluster();
		}
	}

	// Shows the previous relevant cluster in the middle of the view.
	fn prev_cluster(&mut self) {
		let Some(analysis) = &self.analysis else {
			return;
		};

		let count = analysis.relevant_clusters.len();

		if count == 0 {
			return;
		}

		let index = match self.cluster_index {
			Some(index) => (index + count - 1) % count,
			None => count - 1,
		};

		self.show_cluster(index);
	}

	// Shows the next relevant cluster in the middle of the view.
	fn next_cluster(&mut self) {
		let Some(analysis) = &self.analysis else {
			return;
		};

		let count = analysis.relevant_clusters.len();

		if count == 0 {
			return;
		}

		let index = match self.cluster_index {
			Some(index) => (index + 1) % count,
			None => 0,
		};

		self.show_cluster(index);
	}

	// Shows the relevant cluster at the given index in the middle of the view.
	// The view is zoomed in, when the cluster is too small to be seen.
	fn show_cluster(&mut self, index: usize) {
		let Some(analysis) = &self.analysis else {
			return;
		};

		let Some(cluster) = analysis.relevant_clusters.get(index) else {
			return;
		};

		let Some(image) = &self.image else {
			return;
		};

		let (Some(display), Some(view)) = (self.display_rect, self.view_rect) else {
			return;
		};

		let (min_x, min_y, max_x, max_y) = cluster.rect;

		// The center of the cluster, in image pixels:
		let center_x = (min_x + max_x) as f32 / 2.0;

		let center_y = (min_y + max_y) as f32 / 2.0;

		// The size of the cluster with a margin, in image pixels:
		let size_x = (max_x - min_x + 1) as f32 + 2.0 * CLUSTER_MARGIN;

		let size_y = (max_y - min_y + 1) as f32 + 2.0 * CLUSTER_MARGIN;

		// The scale, screen points per image pixel, at which the cluster is
		// clearly visible:
		let scale = (view.width() * 0.5 / size_x).min(view.height() * 0.5 / size_y);

		// The scale of the image at zoom = 1:
		let fit_scale = self.pixel_scale / self.zoom;

		if fit_scale > 0.0 {
			// The view is never zoomed out:
			let zoom = (scale / fit_scale).clamp(0.05, 128.0).max(self.zoom);

			// The rect the image fills at zoom = 1:
			let fitted_min = display.min - self.pan;

			let fitted_size = display.size() / self.zoom;

			// The pan that puts the cluster in the middle of the view:
			self.pan.x = view.center().x
				- fitted_min.x
				- center_x / image.width as f32 * fitted_size.x * zoom;

			self.pan.y = view.center().y
				- fitted_min.y
				- center_y / image.height as f32 * fitted_size.y * zoom;

			self.zoom = zoom;
		}

		self.cluster_index = Some(index);

		// The caption above the cluster, with its number and name:
		self.cluster_caption = Some(ClusterCaption {
			index,
			total: analysis.relevant_clusters.len(),
			name: cluster.name.clone(),
			rect: cluster.rect,
		});

		self.status =
			format!("Relevant cluster #{} of {}", index + 1, analysis.relevant_clusters.len());
	}

	// Draws the caption above the relevant cluster that is shown: its number
	// and its name.
	fn draw_cluster_caption(
		&self,
		ui: &egui::Ui,
		display: egui::Rect,
		size: egui::Vec2,
		area: egui::Rect,
	) {
		let Some(caption) = &self.cluster_caption else {
			return;
		};

		// The position of an image pixel on the screen:
		let to_screen = |x: f32, y: f32| {
			egui::pos2(
				display.min.x + x / size.x * display.width(),
				display.min.y + y / size.y * display.height(),
			)
		};

		let (min_x, min_y, max_x, max_y) = caption.rect;

		// The rectangle of the cluster on the screen:
		let cluster = egui::Rect::from_min_max(
			to_screen(min_x as f32, min_y as f32),
			to_screen(max_x as f32 + 1.0, max_y as f32 + 1.0),
		);

		let text = format!("#{} of {}  {}", caption.index + 1, caption.total, caption.name);

		let font = egui::FontId::proportional(14.0);

		// The caption stays inside the view, it never reaches the scrollbars:
		let painter = ui.painter().with_clip_rect(area);

		let galley = painter.layout_no_wrap(text, font, egui::Color32::WHITE);

		// The caption is above the cluster, but always inside the view:
		let position = egui::pos2(
			(cluster.center().x - galley.size().x / 2.0).clamp(area.min.x, area.max.x),
			(cluster.min.y - galley.size().y - 8.0).max(area.min.y + 4.0),
		);

		let rect = egui::Rect::from_min_size(position, galley.size());

		painter.rect_filled(rect.expand(4.0), 3.0, egui::Color32::from_black_alpha(200));

		painter.galley(rect.min, galley, egui::Color32::WHITE);
	}

	// Draws the scrollbars of the image view. They appear when the image does
	// not fit into the view; dragging their thumb pans the image.
	fn show_scrollbars(
		&mut self,
		ui: &mut egui::Ui,
		display: egui::Rect,
		area: egui::Rect,
		horizontal: bool,
		vertical: bool,
	) {
		if horizontal {
			let bar = egui::Rect::from_min_size(
				egui::pos2(area.min.x, area.max.y),
				egui::vec2(area.width(), SCROLLBAR),
			);

			self.scrollbar(ui, bar, display.min.x, area.min.x, display.width(), area.width(), true);
		}

		if vertical {
			let bar = egui::Rect::from_min_size(
				egui::pos2(area.max.x, area.min.y),
				egui::vec2(SCROLLBAR, area.height()),
			);

			self.scrollbar(
				ui,
				bar,
				display.min.y,
				area.min.y,
				display.height(),
				area.height(),
				false,
			);
		}
	}

	// Draws one scrollbar of the image view and pans the image when its thumb
	// is dragged.
	fn scrollbar(
		&mut self,
		ui: &mut egui::Ui,
		bar: egui::Rect,
		content_min: f32,
		view_min: f32,
		content_size: f32,
		view_size: f32,
		horizontal: bool,
	) {
		let painter = ui.painter();

		// The length of the bar:
		let length = if horizontal { bar.width() } else { bar.height() };

		// The share of the image that is visible:
		let visible = (view_size / content_size).clamp(0.0, 1.0);

		// The thumb of the scrollbar:
		let thumb_size = (length * visible).max(24.0);

		// The share of the image that is scrolled out:
		let scroll = if content_size > view_size {
			((view_min - content_min) / (content_size - view_size)).clamp(0.0, 1.0)
		} else {
			0.0
		};

		let thumb_min = if horizontal {
			bar.min.x + scroll * (length - thumb_size)
		} else {
			bar.min.y + scroll * (length - thumb_size)
		};

		let thumb = if horizontal {
			egui::Rect::from_min_size(
				egui::pos2(thumb_min, bar.min.y + 2.0),
				egui::vec2(thumb_size, bar.height() - 4.0),
			)
		} else {
			egui::Rect::from_min_size(
				egui::pos2(bar.min.x + 2.0, thumb_min),
				egui::vec2(bar.width() - 4.0, thumb_size),
			)
		};

		painter.rect_filled(bar, 2.0, egui::Color32::from_gray(225));

		painter.rect_filled(thumb, 2.0, egui::Color32::from_gray(150));

		// Dragging the thumb scrolls the image:
		let response = ui.allocate_rect(bar, egui::Sense::click_and_drag());

		if response.dragged() {
			self.cluster_caption = None;

			let Some(pointer) = response.interact_pointer_pos() else {
				return;
			};

			let position = if horizontal { pointer.x - bar.min.x } else { pointer.y - bar.min.y };

			let scroll = ((position - thumb_size / 2.0) / (length - thumb_size)).clamp(0.0, 1.0);

			// The position of the image, so that the scroll is at the pointer:
			let new_min = view_min - scroll * (content_size - view_size);

			let delta = new_min - content_min;

			if horizontal {
				self.pan.x += delta;
			} else {
				self.pan.y += delta;
			}
		}
	}

	// Shows the thumbnail of the image above the Detector Rim control. The
	// rectangle in it marks the part of the image that is visible now.
	// Shows the thumbnail of the image with the visible part of it marked.
	// Returns the rect of the thumbnail.
	fn show_thumbnail(&mut self, ui: &mut egui::Ui) -> egui::Rect {
		let Some(image) = &self.image else {
			return egui::Rect::NOTHING;
		};

		let size = egui::vec2(image.width as f32, image.height as f32);

		// The space of the thumbnail in the panel:
		let available = ui.available_size().max(egui::Vec2::ZERO);

		// The thumbnail is at most 240 points wide and 200 points high, so it
		// fits a tall image as well. It never sticks out of the panel:
		let scale = (240.0 / size.x).min(200.0 / size.y).min(available.x / size.x).max(0.0_f32);

		let width = size.x * scale;

		let height = size.y * scale;

		// A collapsed panel has no room for the thumbnail:
		if width <= 0.0 || height <= 0.0 {
			return egui::Rect::NOTHING;
		}

		// The thumbnail is centered in the panel, so that a tall image does not
		// leave it at the left edge:
		let rect = egui::Rect::from_center_size(
			egui::pos2(ui.cursor().min.x + available.x / 2.0, ui.cursor().min.y + height / 2.0),
			egui::vec2(width, height),
		);

		// The response is the one of the thumbnail itself:
		let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

		draw_image(ui.painter(), &self.textures, size, rect);

		let Some(display) = self.display_rect else {
			return rect;
		};

		let view = self.view_rect.unwrap_or(display);

		// The visible part of the image:
		let visible = display.intersect(view);

		if visible.width() <= 0.0 || visible.height() <= 0.0 {
			return rect;
		}

		// The rectangle of the visible area, in the thumbnail:
		let to_thumbnail = |point: egui::Pos2| {
			egui::pos2(
				rect.min.x + (point.x - display.min.x) / display.width() * rect.width(),
				rect.min.y + (point.y - display.min.y) / display.height() * rect.height(),
			)
		};

		let marker = egui::Rect::from_min_max(to_thumbnail(visible.min), to_thumbnail(visible.max));

		ui.painter().rect_stroke(
			marker,
			0.0,
			egui::Stroke::new(1.5, egui::Color32::RED),
			egui::StrokeKind::Middle,
		);

		// Clicking the thumbnail moves the view to that part of the image:
		if response.clicked() || response.dragged() {
			self.cluster_caption = None;

			let Some(pointer) = response.interact_pointer_pos() else {
				return rect;
			};

			let u = ((pointer.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);

			let v = ((pointer.y - rect.min.y) / rect.height()).clamp(0.0, 1.0);

			// The position of the pointer in the image, on the screen:
			let x = display.min.x + u * display.width();

			let y = display.min.y + v * display.height();

			// The view is centered on that point:
			self.pan.x += view.center().x - x;

			self.pan.y += view.center().y - y;
		}

		rect
	}

	// Shows the Detector Rim control above the Report panel: the width of the
	// border that is cut from the active area of the detector.
	fn show_detector_rim(&mut self, ui: &mut egui::Ui) {
		let tooltip = "The width of the detector rim, pixels. The border of the image of \
		               this width is cut from the active area of the detector: its pixels \
		               are not evaluated and it is the edge of the detector.";

		// The rim must leave at least one pixel of the active area:
		let max = self
			.image
			.as_ref()
			.map(|image| (image.width.min(image.height) / 2).saturating_sub(1))
			.unwrap_or(0);

		if self.detector_rim > max {
			self.detector_rim = max;
		}

		let mut changed = false;

		ui.horizontal(|ui| {
			ui.label("Detector Rim").on_hover_text(tooltip);

			let response = ui
				.add_sized(
					[100.0, 22.0],
					egui::DragValue::new(&mut self.detector_rim).speed(1.0).range(0..=max),
				)
				.on_hover_text(tooltip);

			changed = response.changed();
		});

		if changed {
			self.apply_rim(ui.ctx());
		}
	}

	// Re-analyzes the image with the current detector rim and rebuilds the
	// preview texture.
	fn apply_rim(&mut self, ctx: &egui::Context) {
		let Some(image) = &self.image else {
			return;
		};

		let analysis = analyze(image, self.detector_rim);

		let textures = make_textures(ctx, &analysis, image.width, image.height);

		let report = report_text(image, &analysis);

		self.report = report;

		self.analysis = Some(analysis);

		self.textures = textures;
	}

	// Shows the Report panel (right): the thumbnail, the detector rim, the four
	// indicators, the list of the relevant clusters and the Generate Report
	// button.
	fn show_report(&mut self, ui: &mut egui::Ui) -> Option<MenuCommand> {
		self.show_thumbnail(ui);

		ui.add_space(6.0);

		self.show_detector_rim(ui);

		ui.separator();

		ui.label(egui::RichText::new("Report").strong().size(14.0));

		ui.add_space(8.0);

		let (single, irrelevant, lines, relevant) = match &self.analysis {
			Some(analysis) => (
				analysis.single_bad_pixels,
				analysis.irrelevant_clusters.len(),
				analysis.line_segments.len(),
				analysis.relevant_clusters.len(),
			),
			None => (0, 0, 0, 0),
		};

		// 2-column grid keeps the labels left-aligned and all the numbers on
		// one vertical line:
		egui::Grid::new("report").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
			indicator(ui, "Single Bad Pixels", single);

			ui.end_row();

			indicator(ui, "Irrelevant Clusters", irrelevant);

			ui.end_row();

			indicator(ui, "Line Segments", lines);

			ui.end_row();

			indicator(ui, "Relevant Clusters", relevant);

			ui.end_row();
		});

		ui.add_space(8.0);

		ui.separator();

		// The text of the report:
		if self.analysis.is_some() {
			let report_height = (ui.available_height() * 0.3).min(220.0).max(90.0);

			egui::ScrollArea::vertical()
				.id_salt("report_text")
				.max_height(report_height)
				.auto_shrink([false, false])
				.show(ui, |ui| {
					ui.add(
						egui::Label::new(egui::RichText::new(&self.report).monospace().size(11.0))
							.wrap(),
					);
				});

			ui.add_space(6.0);
		}

		ui.separator();

		// The list of the relevant clusters scrolls; the buttons sit right under
		// the scroll bar. The space of the row of the buttons is reserved:
		let height = (ui.available_height() - 44.0).max(60.0);

		egui::ScrollArea::vertical()
			.id_salt("relevant_clusters")
			.max_height(height)
			.auto_shrink([false, false])
			.show(ui, |ui| match &self.analysis {
				Some(analysis) if !analysis.relevant_clusters.is_empty() => {
					for cluster in &analysis.relevant_clusters {
						ui.label(cluster.name.clone());

						let positions: Vec<String> = cluster
							.ckp_positions
							.iter()
							.map(|(col, row)| format!("(Col {col}, Row {row})"))
							.collect();

						ui.weak(positions.join("  "));

						ui.add_space(6.0);
					}
				}
				_ => {
					ui.weak("Open an image to see the relevant clusters");
				}
			});

		ui.add_space(6.0);

		// The three buttons of the report in one row: save the report in a
		// file, copy it in the clipboard, view it in a window.
		let mut command = None;

		ui.horizontal(|ui| {
			let width =
				((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(0.0_f32);

			if ui.add_sized([width, 30.0], egui::Button::new("Save")).clicked() {
				command = Some(MenuCommand::GenerateReport);
			}

			if ui.add_sized([width, 30.0], egui::Button::new("Copy")).clicked() {
				command = Some(MenuCommand::CopyReport);
			}

			if ui.add_sized([width, 30.0], egui::Button::new("View")).clicked() {
				command = Some(MenuCommand::ViewReport);
			}
		});

		command
	}

	// The kind of the pixel at (x, y): 'D' for a bad pixel, 'K' for a cluster
	// kernel pixel and 'C' for a good pixel that has a bad neighbour.
	fn pixel_kind_at(&self, x: usize, y: usize) -> Option<char> {
		let analysis = self.analysis.as_ref()?;

		let image = self.image.as_ref()?;

		if x >= image.width || y >= image.height {
			return None;
		}

		let index = y * image.width + x;

		// The detector rim is not evaluated:
		if analysis.rim[index] {
			return None;
		}

		if analysis.ckp[index] {
			return Some('K');
		}

		if analysis.bad[index] {
			return Some('D');
		}

		// A good pixel next to a bad one:
		for dy in -1i32..=1 {
			for dx in -1i32..=1 {
				if dx == 0 && dy == 0 {
					continue;
				}

				let nx = x as i32 + dx;

				let ny = y as i32 + dy;

				if nx < 0 || ny < 0 || nx as usize >= image.width || ny as usize >= image.height {
					continue;
				}

				if analysis.bad[ny as usize * image.width + nx as usize] {
					return Some('C');
				}
			}
		}

		None
	}

	// The kind of the pixel under the mouse pointer: "D" for a bad pixel, "K"
	// for a cluster kernel pixel and "C" for a good pixel that has a bad
	// neighbour.
	fn pixel_kind(&self) -> Option<(char, egui::Color32)> {
		let (x, y) = self.hover_pixel?;

		let kind = self.pixel_kind_at(x, y)?;

		let color = match kind {
			'K' => egui::Color32::RED,
			'D' => egui::Color32::from_rgb(0x20, 0x20, 0x20),
			_ => egui::Color32::GRAY,
		};

		Some((kind, color))
	}

	// Draws the grid between the pixels and the letter of every pixel, when the
	// pixels are big enough on the screen.
	fn draw_pixel_details(&self, ui: &egui::Ui, display: egui::Rect, view: egui::Rect, cell: f32) {
		// The grid and the letters stay inside the view, they never reach the
		// scrollbars:
		let painter = ui.painter().with_clip_rect(view);

		// The visible part of the image:
		let visible = display.intersect(view);

		if visible.width() <= 0.0 || visible.height() <= 0.0 {
			return;
		}

		// The grid between the pixels:
		let stroke = egui::Stroke::new(1.0, egui::Color32::GRAY);

		let first = ((visible.min.x - display.min.x) / cell).floor() as i64;

		let last = ((visible.max.x - display.min.x) / cell).ceil() as i64;

		for index in first..=last {
			let x = display.min.x + index as f32 * cell;

			painter
				.line_segment([egui::pos2(x, visible.min.y), egui::pos2(x, visible.max.y)], stroke);
		}

		let first = ((visible.min.y - display.min.y) / cell).floor() as i64;

		let last = ((visible.max.y - display.min.y) / cell).ceil() as i64;

		for index in first..=last {
			let y = display.min.y + index as f32 * cell;

			painter
				.line_segment([egui::pos2(visible.min.x, y), egui::pos2(visible.max.x, y)], stroke);
		}

		let Some(image) = &self.image else {
			return;
		};

		// The letter in the middle of every pixel:
		let font = egui::FontId::proportional(cell * 0.55);

		let first_x = ((visible.min.x - display.min.x) / cell).floor().max(0.0) as usize;

		let first_y = ((visible.min.y - display.min.y) / cell).floor().max(0.0) as usize;

		let last_x =
			(((visible.max.x - display.min.x) / cell).ceil().max(0.0) as usize).min(image.width);

		let last_y =
			(((visible.max.y - display.min.y) / cell).ceil().max(0.0) as usize).min(image.height);

		for y in first_y..last_y {
			for x in first_x..last_x {
				let Some(kind) = self.pixel_kind_at(x, y) else {
					continue;
				};

				// The letter is drawn on the pixel, so it must contrast with it:
				// a white letter on a good pixel, a yellow one on a cluster
				// kernel pixel and a black one on a bad pixel.
				let color = match kind {
					'K' => egui::Color32::YELLOW,
					'D' => egui::Color32::BLACK,
					_ => egui::Color32::WHITE,
				};

				let center = egui::pos2(
					display.min.x + (x as f32 + 0.5) * cell,
					display.min.y + (y as f32 + 0.5) * cell,
				);

				painter.text(
					center,
					egui::Align2::CENTER_CENTER,
					kind.to_string(),
					font.clone(),
					color,
				);
			}
		}
	}

	// Shows the status bar (bottom): the image, the mouse position, the zoom
	// ratio and the last action result.
	fn show_status(&mut self, ui: &mut egui::Ui) {
		ui.horizontal(|ui| {
			match &self.image {
				Some(image) => ui.label(format!(
					"Image: {} ({} × {})",
					self.image_name, image.width, image.height
				)),
				None => ui.weak("No image opened"),
			};

			if self.image.is_some() {
				ui.separator();

				// The position shown in the status bar: when a relevant cluster
				// is shown (via the arrows, the buttons or the thumbnail), the
				// top-left corner of its rectangle; otherwise the position of
				// the mouse pointer:
				let status_position = self.status_position();

				// The coordinates of the position:
				match status_position {
					Some((x, y)) => {
						ui.label(egui::RichText::new(format!("X: {x}  Y: {y}")).monospace())
					}
					None => ui.weak("X: —  Y: —"),
				};

				// The kind of the pixel at the shown position: "D" for a bad
				// pixel, "K" for a cluster kernel pixel and "C" for a good pixel
				// next to a bad one:
				if let Some((x, y)) = status_position {
					if let Some(kind) = self.pixel_kind_at(x, y) {
						let color = match kind {
							'K' => egui::Color32::RED,
							'D' => egui::Color32::from_rgb(0x20, 0x20, 0x20),
							_ => egui::Color32::GRAY,
						};

						ui.label(
							egui::RichText::new(kind.to_string()).monospace().strong().color(color),
						);
					}
				}

				ui.separator();

				// The zoom ratio and the scale of the display, screen pixels per
				// image pixel:
				ui.label(format!("Zoom: {:.2}x ({:.1} px/px)", self.zoom, self.pixel_scale));
			}

			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				ui.label(&self.status);
			});
		});
	}

	// The position shown in the status bar: when a relevant cluster is shown
	// (via the arrows, the buttons or the thumbnail), the top-left corner of
	// its rectangle; otherwise the position of the mouse pointer.
	fn status_position(&self) -> Option<(usize, usize)> {
		match &self.cluster_caption {
			Some(caption) => Some((caption.rect.0, caption.rect.1)),
			None => self.hover_pixel,
		}
	}

	// Opens the files dropped on the window.
	fn open_dropped_files(&mut self, ctx: &egui::Context) {
		let dropped: Vec<egui::DroppedFileHandle> =
			ctx.input(|input| input.raw.dropped_files.clone());

		self.load_dropped(ctx, dropped);
	}

	// Opens the given dropped files.
	fn load_dropped(&mut self, ctx: &egui::Context, dropped: Vec<egui::DroppedFileHandle>) {
		for file in dropped {
			let name = file
				.path()
				.file_name()
				.map(|name| name.to_string_lossy().to_string())
				.unwrap_or_else(|| "image".to_owned());

			#[cfg(not(target_arch = "wasm32"))]
			{
				// The image is remembered for the next time:
				last_image::remember(file.path());

				match file.bytes() {
					Ok(bytes) => self.load(&name, &bytes, ctx),
					Err(error) => self.status = format!("Read error: {error}"),
				}
			}

			// In the browser the file is read asynchronously:
			#[cfg(target_arch = "wasm32")]
			{
				let pending = self.pending_files.clone();

				wasm_bindgen_futures::spawn_local(async move {
					let result = file.bytes_async().await.map(|bytes| (name, bytes));

					pending.borrow_mut().push(result);
				});
			}
		}
	}

	// Picks an image file (native version) and opens it.
	#[cfg(not(target_arch = "wasm32"))]
	fn open_image(&mut self, ctx: &egui::Context) {
		let mut dialog =
			rfd::FileDialog::new().add_filter("Images", &["tif", "tiff", "png", "dcm"]);

		// The dialog opens in the directory of the last opened image:
		if let Some(directory) = last_image::directory() {
			dialog = dialog.set_directory(directory);
		}

		let Some(path) = dialog.pick_file() else {
			return;
		};

		// The image is remembered for the next time:
		last_image::remember(&path);

		let name = path
			.file_name()
			.map(|name| name.to_string_lossy().to_string())
			.unwrap_or_else(|| "image".to_owned());

		match std::fs::read(&path) {
			Ok(bytes) => self.load(&name, &bytes, ctx),
			Err(error) => self.status = format!("Read error: {error}"),
		}
	}

	// Picks an image file (web version) and opens it. The file is read in the
	// background, the result is picked up by the next frame.
	#[cfg(target_arch = "wasm32")]
	fn open_image(&mut self, ctx: &egui::Context) {
		use eframe::wasm_bindgen::JsCast as _;

		let document = web_sys::window().unwrap().document().unwrap();

		if self.file_input.is_none() {
			let input: web_sys::HtmlInputElement =
				document.create_element("input").unwrap().dyn_into().unwrap();

			input.set_type("file");

			input.set_accept("image/tiff,image/png,.tif,.tiff,.png,.dcm");

			input.style().set_property("display", "none").unwrap();

			document.body().unwrap().append_child(&input).unwrap();

			let pending = self.pending_files.clone();

			let handler = eframe::wasm_bindgen::closure::Closure::wrap(Box::new(
				move |event: web_sys::Event| {
					let Some(target) = event.target() else {
						return;
					};

					let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() else {
						return;
					};

					let Some(file) = input.files().and_then(|files| files.get(0)) else {
						return;
					};

					let name = file.name();

					let pending = pending.clone();

					wasm_bindgen_futures::spawn_local(async move {
						let result = read_file(&file).await.map(|bytes| (name, bytes));

						pending.borrow_mut().push(result);
					});
				},
			)
				as Box<dyn FnMut(web_sys::Event)>);

			input.set_onchange(Some(handler.as_ref().unchecked_ref()));

			self.file_handler = Some(handler);

			self.file_input = Some(input);
		}

		let input = self.file_input.as_ref().unwrap();

		// Clear the value, so that picking the same file again fires the event:
		input.set_value("");

		input.click();

		let _ = ctx;
	}

	// Loads the image from the file contents and analyzes it.
	fn load(&mut self, name: &str, bytes: &[u8], ctx: &egui::Context) {
		match load_image(name, bytes) {
			Ok(image) => {
				let size = format!("{} × {}", image.width, image.height);

				self.set_image(name.to_owned(), image, ctx);

				self.status = format!("Opened {name} ({size})");
			}
			Err(error) => self.status = error,
		}
	}

	// Shows the full report in a modal window, about the size of the app.
	fn show_report_window(&mut self, ctx: &egui::Context) {
		let screen = ctx.viewport_rect().size();

		let mut close = false;

		let response = egui::Modal::new(egui::Id::new("report_window")).show(ctx, |ui| {
			ui.set_width((screen.x * 0.9).min(1600.0));

			ui.set_max_height((screen.y * 0.85).max(200.0));

			ui.label(egui::RichText::new("Bad Pixel Analysis Report").strong().size(16.0));

			ui.separator();

			// The report scrolls in the rest of the window:
			let height = (ui.available_height() - 40.0).max(100.0);

			egui::ScrollArea::vertical()
				.id_salt("report_window_text")
				.max_height(height)
				.auto_shrink([false, false])
				.show(ui, |ui| {
					ui.monospace(&self.report);
				});

			ui.add_space(8.0);

			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				if ui.button("Close").clicked() {
					close = true;
				}
			});
		});

		if close || response.should_close() {
			self.show_report_window = false;
		}
	}

	// Builds the report text and saves it.
	fn generate_report(&mut self) {
		let (Some(image), Some(analysis)) = (&self.image, &self.analysis) else {
			self.status = "Open an image first".to_owned();

			return;
		};

		let text = report_text(image, analysis);

		match save_report(&text) {
			Ok(Some(message)) => self.status = message,
			Ok(None) => {}
			Err(error) => self.status = format!("Report save error: {error}"),
		}
	}

	// Builds the report text and copies it to the clipboard.
	fn copy_report(&mut self, ctx: &egui::Context) {
		let (Some(image), Some(analysis)) = (&self.image, &self.analysis) else {
			self.status = "Open an image first".to_owned();

			return;
		};

		let text = report_text(image, analysis);

		ctx.copy_text(text);

		self.status = "Report copied to the clipboard".to_owned();
	}
}

// The path of the last opened image, remembered in the registry of the user
// (desktop version only).
#[cfg(not(target_arch = "wasm32"))]
mod last_image {
	use std::path::{Path, PathBuf};

	use winreg::RegKey;
	use winreg::enums::HKEY_CURRENT_USER;

	// The key of the app in the registry of the user:
	const KEY: &str = r"Software\ASTM";

	// The name of the value with the path of the last opened image:
	const VALUE: &str = "last_badpix_image";

	// The path of the last opened image, if there is one.
	pub fn path() -> Option<PathBuf> {
		let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok()?;

		let path: String = key.get_value(VALUE).ok()?;

		if path.is_empty() {
			return None;
		}

		Some(PathBuf::from(path))
	}

	// The directory of the last opened image, if there is one.
	pub fn directory() -> Option<PathBuf> {
		let path = path()?;

		// The directory of the image:
		let directory = path.parent()?;

		if directory.as_os_str().is_empty() {
			return None;
		}

		Some(directory.to_path_buf())
	}

	// Remembers the path of the opened image. The key is created when it does
	// not exist yet.
	pub fn remember(path: &Path) {
		// Only the absolute paths are of use:
		if !path.is_absolute() {
			return;
		}

		let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) else {
			return;
		};

		let _ = key.set_value(VALUE, &path.to_string_lossy().to_string());
	}
}

// One indicator of the Report panel: a label and the value in a box.
fn indicator(ui: &mut egui::Ui, label: &str, value: usize) {
	ui.label(label);

	egui::Frame::group(ui.style()).show(ui, |ui| {
		ui.allocate_ui_with_layout(
			egui::Vec2::new(60.0, 20.0),
			egui::Layout::right_to_left(egui::Align::Center),
			|ui| {
				ui.monospace(format!("{value}"));
			},
		);
	});
}

// Reads the contents of a file picked in the browser.
#[cfg(target_arch = "wasm32")]
async fn read_file(file: &web_sys::File) -> Result<Vec<u8>, String> {
	use wasm_bindgen_futures::JsFuture;

	let reader = web_sys::FileReader::new().map_err(|error| format!("Read error: {error:?}"))?;

	// The promise is resolved by the reader when the file is read:
	let promise = js_sys::Promise::new(&mut |resolve, reject| {
		reader.set_onload(Some(&resolve));

		reader.set_onerror(Some(&reject));
	});

	reader.read_as_array_buffer(file).map_err(|error| format!("Read error: {error:?}"))?;

	JsFuture::from(promise).await.map_err(|error| format!("Read error: {error:?}"))?;

	let buffer = reader.result().map_err(|error| format!("Read error: {error:?}"))?;

	Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

impl eframe::App for App {
	fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
		// App title (web only — on desktop the OS window title serves this purpose):
		#[cfg(target_arch = "wasm32")]
		egui::Panel::top(egui::Id::new("title_bar")).frame(light_panel_frame()).show(ui, |ui| {
			ui.label(egui::RichText::new("Bad Pixel Analyzer").strong().size(18.0));

			ui.weak(
				"Experimental tool for test purposes only. A client-side app: the files are \
				 processed locally in your browser and are never uploaded to any server.",
			);
		});

		// Toolbar (menu):
		let toolbar = egui::Panel::top(egui::Id::new("toolbar"))
			.frame(light_panel_frame())
			.show(ui, |ui| self.show_menu(ui));

		let menu_command = toolbar.inner;

		// Tools (left) — width fits the 36 px icon buttons plus the 8 px panel margins,
		// so the gaps left and right of each button match:
		egui::Panel::left(egui::Id::new("tools"))
			.frame(light_panel_frame())
			.exact_size(52.0)
			.show(ui, |ui| self.show_tools(ui));

		// Report (right) — its Generate Report button yields a command like the menu does:
		// The panel is at least 205 points wide, so that the four indicators and
		// the row of three buttons always fit; it can still be made wider.
		let report_command = egui::Panel::right(egui::Id::new("report"))
			.frame(light_panel_frame())
			.min_size(205.0)
			.default_size(205.0)
			.show(ui, |ui| self.show_report(ui))
			.inner;

		let menu_command = menu_command.or(report_command);

		// Status bar:
		egui::Panel::bottom(egui::Id::new("status_bar"))
			.frame(light_panel_frame())
			.show(ui, |ui| self.show_status(ui));

		// The files picked in the browser or dropped on the page are read in the
		// background:
		#[cfg(target_arch = "wasm32")]
		{
			let files: Vec<Result<(String, Vec<u8>), String>> =
				self.pending_files.borrow_mut().drain(..).collect();

			for result in files {
				match result {
					Ok((name, bytes)) => self.load(&name, &bytes, ui.ctx()),
					Err(error) => self.status = error,
				}
			}
		}

		// The files dropped on the window:
		self.open_dropped_files(ui.ctx());

		// Main content — the image, with wheel zoom (anchored at the pointer) and pan:
		egui::CentralPanel::default().frame(light_panel_frame()).show(ui, |ui| {
			ui.style_mut().visuals = egui::Visuals::light();

			// Clone handles the borrow: the view state below is mutated while the
			// textures are used for painting.
			let Some(image) = &self.image else {
				ui.centered_and_justified(|ui| {
					ui.weak("Open an image (Ctrl+O) or drop an image file here to analyze it.");
				});

				return;
			};

			let view_rect = ui.available_rect_before_wrap();

			// The splitter between the image and the report panel can shrink the
			// image area to zero width. Then there is nothing to paint, so return
			// before any zero-size texture is drawn to the renderer.
			if view_rect.width() <= 0.0 || view_rect.height() <= 0.0 {
				return;
			};

			// The size of the image:
			let size = egui::vec2(image.width as f32, image.height as f32);

			// The scale of the image at zoom = 1:
			let fit_scale = (view_rect.width() / size.x).min(view_rect.height() / size.y);

			// The scrollbars appear when the image does not fit into the view:
			let zoomed = size * fit_scale * self.zoom;

			let horizontal = zoomed.x > view_rect.width();

			let vertical = zoomed.y > view_rect.height();

			// The area of the image, without the scrollbars:
			let mut area = view_rect;

			if horizontal {
				area.max.y -= SCROLLBAR;
			}

			if vertical {
				area.max.x -= SCROLLBAR;
			}

			let response = ui.allocate_rect(area, egui::Sense::click_and_drag());

			// Rect the image fills at zoom = 1 (fit to screen, centered):
			let fit_scale = (area.width() / size.x).min(area.height() / size.y);

			let fitted = egui::Rect::from_center_size(area.center(), size * fit_scale);

			// The scale at zoom = 1 is needed for the actual size (1:1):
			self.fit_scale = fit_scale;

			// Zoom requested by the Tools buttons (around the view center):
			if let Some(factor) = self.pending_zoom.take() {
				self.cluster_caption = None;

				self.zoom_step(fitted, area.center(), factor);
			}

			// Zoom on scroll, anchored at the pointer:
			let scroll: f32 = ui.input(|input| input.smooth_scroll_delta.y);

			if response.hovered() && scroll != 0.0 {
				self.cluster_caption = None;

				let anchor = ui.input(|input| input.pointer.hover_pos()).unwrap_or(area.center());

				self.zoom_step(fitted, anchor, (1.0 + scroll * 0.005).clamp(0.7, 1.3));
			}

			// Pan by dragging with the mouse:
			if response.dragged() {
				self.cluster_caption = None;

				self.pan += response.drag_delta();
			}

			if response.dragged() {
				ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
			}

			// The image must stay visible in the view:
			self.limit_pan(fitted, area);

			let display =
				egui::Rect::from_min_size(fitted.min + self.pan, fitted.size() * self.zoom);

			// The scale of the display, screen pixels per image pixel:
			self.pixel_scale = display.width() / size.x;

			// The pixel under the mouse pointer:
			self.hover_pixel = None;

			if let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) {
				if display.contains(pointer) {
					let x = ((pointer.x - display.min.x) / display.width() * size.x).floor();

					let y = ((pointer.y - display.min.y) / display.height() * size.y).floor();

					if x >= 0.0
						&& y >= 0.0 && (x as usize) < size.x as usize
						&& (y as usize) < size.y as usize
					{
						self.hover_pixel = Some((x as usize, y as usize));
					}
				}
			}

			// The image, clipped to the area without the scrollbars:
			let painter = ui.painter().with_clip_rect(area);

			draw_image(&painter, &self.textures, size, display);

			// The grid between the pixels and the letter of every pixel, when the
			// pixels are big enough on the screen:
			if self.pixel_scale >= PIXEL_DETAIL_SCALE {
				self.draw_pixel_details(ui, display, area, self.pixel_scale);
			}

			// The caption above the relevant cluster that is shown:
			self.draw_cluster_caption(ui, display, size, area);

			// The scrollbars of the image view, above the image, the letters and
			// the caption:
			self.show_scrollbars(ui, display, area, horizontal, vertical);

			// The rectangles, for the thumbnail of the Report panel:
			self.display_rect = Some(display);

			self.view_rect = Some(area);
		});

		if let Some(command) = menu_command {
			match command {
				MenuCommand::OpenImage => {
					self.open_image(ui.ctx());
				}

				MenuCommand::GenerateReport => {
					self.generate_report();
				}

				MenuCommand::CopyReport => {
					self.copy_report(ui.ctx());
				}

				MenuCommand::ViewReport => {
					self.show_report_window = true;
				}

				#[cfg(not(target_arch = "wasm32"))]
				MenuCommand::Quit => {
					ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
				}
			}
		}

		if self.show_report_window {
			self.show_report_window(ui.ctx());
		}

		if self.show_about {
			let response = egui::Modal::new(egui::Id::new("about")).show(ui.ctx(), |ui| {
				ui.set_width(280.0);

				ui.vertical_centered(|ui| {
					ui.heading(env!("CARGO_PKG_NAME"));

					ui.weak(concat!("v", env!("CARGO_PKG_VERSION")));

					ui.weak("Bad pixel analyzer for ASTM E2597-22");

					ui.weak("Analyzes TIFF, PNG and DICONDE detector images");
				});

				ui.add_space(8.0);

				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					ui.button("OK").clicked()
				})
				.inner
			});

			if response.inner || response.should_close() {
				self.show_about = false;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// Builds an image of good pixels with the bad pixels at the given positions.
	fn image_with_bad(width: usize, height: usize, bad: &[(usize, usize)]) -> GrayImage {
		let mut pixels = vec![0u16; width * height];

		for &(x, y) in bad {
			pixels[y * width + x] = 1;
		}

		GrayImage { width, height, pixels }
	}

	#[test]
	fn single_bad_pixel() {
		let image = image_with_bad(10, 10, &[(5, 5)]);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.single_bad_pixels, 1);
		assert_eq!(analysis.irrelevant_clusters.len(), 0);
		assert_eq!(analysis.relevant_clusters.len(), 0);
		assert_eq!(analysis.line_segments.len(), 0);
	}

	#[test]
	fn irrelevant_cluster() {
		// A 2×2 block: every pixel still has five good neighbours.
		let image = image_with_bad(10, 10, &[(4, 4), (5, 4), (4, 5), (5, 5)]);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.single_bad_pixels, 0);
		assert_eq!(analysis.irrelevant_clusters.len(), 1);
		assert_eq!(analysis.irrelevant_clusters[0].name, "2x2 cluster4");
		assert_eq!(analysis.relevant_clusters.len(), 0);
	}

	#[test]
	fn relevant_cluster() {
		// A 3×3 block: the middle pixel and the four pixels around it have
		// less than five good neighbours.
		let bad: Vec<(usize, usize)> = (4..7).flat_map(|y| (4..7).map(move |x| (x, y))).collect();

		let image = image_with_bad(10, 10, &bad);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.relevant_clusters.len(), 1);
		assert_eq!(analysis.relevant_clusters[0].name, "rel3x3 cluster9-5");
		assert_eq!(analysis.irrelevant_clusters.len(), 0);
	}

	#[test]
	fn line_segment() {
		let bad: Vec<(usize, usize)> = (0..20).map(|x| (x, 5)).collect();

		let image = image_with_bad(30, 12, &bad);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.line_segments.len(), 1);
		assert_eq!(analysis.line_segments[0].name, "1x20 Line20");
		assert_eq!(analysis.relevant_clusters.len(), 0);
		assert_eq!(analysis.irrelevant_clusters.len(), 0);
	}

	#[test]
	fn line_segment_with_side_pixels() {
		// Two bad pixels next to the line are a part of the line segment.
		let mut bad: Vec<(usize, usize)> = (0..20).map(|x| (x, 5)).collect();

		bad.push((3, 4));

		bad.push((17, 4));

		let image = image_with_bad(30, 12, &bad);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.line_segments.len(), 1);
		assert_eq!(analysis.line_segments[0].name, "2x20 Line22");
	}

	#[test]
	fn line_segment_is_separated_from_the_cluster_at_its_end() {
		let mut bad: Vec<(usize, usize)> = (0..20).map(|x| (x, 5)).collect();

		bad.extend((20..23).flat_map(|x| (5..8).map(move |y| (x, y))));

		let image = image_with_bad(40, 12, &bad);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.line_segments.len(), 1);
		assert_eq!(analysis.line_segments[0].name, "1x20 Line20");
		assert_eq!(analysis.relevant_clusters.len(), 1);
		assert_eq!(analysis.relevant_clusters[0].name, "rel3x3 cluster9-6");
	}

	#[test]
	fn a_kernel_pixel_of_a_line_at_the_end_of_a_cluster_belongs_to_the_cluster() {
		// A horizontal line with a 2x3 block at its end, at the edge of the
		// detector: the last pixel of the line is a kernel pixel that touches
		// the cluster, so it is cut from the line and is a part of it.
		let mut bad: Vec<(usize, usize)> = (0..20).map(|x| (x, 5)).collect();

		bad.extend([(20, 4), (21, 4), (20, 5), (21, 5), (20, 6), (21, 6)]);

		let image = image_with_bad(22, 12, &bad);

		let analysis = analyze(&image, 0);

		// The line is one pixel shorter:
		assert_eq!(analysis.line_segments.len(), 1);
		assert_eq!(analysis.line_segments[0].name, "1x19 Line19");

		// The cluster is a 3x3 block of 7 kernel pixels:
		assert_eq!(analysis.relevant_clusters.len(), 1);
		assert_eq!(analysis.relevant_clusters[0].name, "rel3x3 cluster7-7");
	}

	#[test]
	fn report_lists_the_defects() {
		let mut bad: Vec<(usize, usize)> = (0..20).map(|x| (x, 5)).collect();

		bad.push((30, 30));

		bad.extend((20..23).flat_map(|x| (5..8).map(move |y| (x, y))));

		let image = image_with_bad(40, 40, &bad);

		let analysis = analyze(&image, 0);

		let text = report_text(&image, &analysis);

		println!("{text}");

		// The summary:
		assert!(text.contains("ASTM Summary\n"));
		assert!(text.contains("Total Defects = 30\n"));
		assert!(text.contains("Defects Area, % = 1,875000\n"));
		assert!(text.contains("Single Bad Pixels = 1\n"));
		assert!(text.contains("Bad Line Segments = 1\n"));
		assert!(text.contains("Irrelevant Clusters = 0\n"));
		assert!(text.contains("Relevant Clusters = 1\n"));
		assert!(text.contains("single bad pixels (1)\n"));

		// The lists of the defects:
		assert!(text.contains("bad line segments (1):\n1x20 Line20\t(Col 0, Row 5)\n"));
		assert!(text.contains("irrelevant clusters (0):\n"));
		assert!(text.contains("relevant clusters (1):\nrel3x3 cluster9-6\t(Col 20, Row 5)\n"));

		// The ASTM 2737 report:
		assert!(text.contains("ASTM 2737 Report for Relevant Clusters\n"));
		assert!(
			text.contains("relevant Cluster #1\t3x3 cluster9-6 No of ClusterKernelPixel = \t6\n")
		);
		assert!(
			text.contains("Position of relevant Cluster #1 Col1\t20\tCol2\t22\tRow1\t5\tRow2\t7\n")
		);
	}

	#[test]
	fn pixel_kind_reports_the_defect_under_the_mouse() {
		// A 3×3 block of bad pixels with a cluster kernel pixel in the middle,
		// and a good pixel next to the block.
		let bad: Vec<(usize, usize)> = (4..7).flat_map(|y| (4..7).map(move |x| (x, y))).collect();

		let image = image_with_bad(10, 10, &bad);

		let mut app = App::default();

		app.image = Some(image);

		app.analysis = Some(analyze(&app.image.as_ref().unwrap(), 0));

		// The middle of the block is a cluster kernel pixel:
		app.hover_pixel = Some((5, 5));

		assert_eq!(app.pixel_kind().map(|(letter, _)| letter), Some('K'));

		// The corner of the block is a bad pixel:
		app.hover_pixel = Some((4, 4));

		assert_eq!(app.pixel_kind().map(|(letter, _)| letter), Some('D'));

		// The good pixel next to the block is next to a bad one:
		app.hover_pixel = Some((3, 5));

		assert_eq!(app.pixel_kind().map(|(letter, _)| letter), Some('C'));

		// A good pixel far from the block:
		app.hover_pixel = Some((0, 0));

		assert_eq!(app.pixel_kind(), None);
	}

	#[test]
	fn status_position_shows_the_cluster_top_left_when_a_cluster_is_shown() {
		// A cluster at columns 20-22, rows 5-7:
		let mut bad: Vec<(usize, usize)> = Vec::new();

		for y in 5..=7 {
			for x in 20..=22 {
				bad.push((x, y));
			}
		}

		let image = image_with_bad(40, 40, &bad);

		let mut app = App::default();

		app.image = Some(image);

		app.analysis = Some(analyze(&app.image.as_ref().unwrap(), 0));

		// Without a shown cluster, the status bar shows the mouse position:
		app.hover_pixel = Some((30, 30));

		assert_eq!(app.status_position(), Some((30, 30)));

		// When a cluster is shown, the status bar shows its top-left corner:
		app.cluster_caption = Some(ClusterCaption {
			index: 0,
			total: 1,
			name: "rel3x3 cluster9-5".to_owned(),
			rect: (20, 5, 22, 7),
		});

		assert_eq!(app.status_position(), Some((20, 5)));

		// When there is no mouse position either, it is None:
		app.cluster_caption = None;

		app.hover_pixel = None;

		assert_eq!(app.status_position(), None);
	}

	#[test]
	fn changing_the_detector_rim_re_analyzes_the_image() {
		let ctx = egui::Context::default();

		let mut app = App::default();

		app.image = Some(image_with_bad(10, 10, &[(1, 1), (5, 5)]));

		app.analysis = Some(analyze(&app.image.as_ref().unwrap(), 0));

		assert_eq!(app.analysis.as_ref().unwrap().single_bad_pixels, 2);

		app.detector_rim = 2;

		app.apply_rim(&ctx);

		assert_eq!(app.analysis.as_ref().unwrap().single_bad_pixels, 1);
	}

	#[test]
	fn the_bad_pixels_of_the_rim_are_shown_but_not_evaluated() {
		let image = image_with_bad(10, 10, &[(1, 1), (5, 5)]);

		let analysis = analyze(&image, 2);

		// The bad pixel of the rim is shown...
		assert!(analysis.rim[1 * 10 + 1]);

		assert!(analysis.rim_bad[1 * 10 + 1]);

		// ... but it is not evaluated:
		assert!(!analysis.bad[1 * 10 + 1]);

		assert_eq!(analysis.single_bad_pixels, 1);

		// The bad pixel of the active area is evaluated as usual:
		assert!(analysis.bad[5 * 10 + 5]);

		assert!(!analysis.rim_bad[5 * 10 + 5]);
	}

	#[test]
	fn the_preview_shows_the_detector_rim_and_the_defects() {
		// A bad pixel in the rim, a good pixel in the rim, a bad pixel in the
		// active area and a cluster kernel pixel:
		let bad = [(1, 1), (5, 5), (6, 6), (7, 7)];

		let image = image_with_bad(10, 10, &bad);

		let analysis = analyze(&image, 2);

		// The bad pixel of the rim is dark gray, the good one is gray:
		assert_eq!(pixel_color(&analysis, 1 * 10 + 1), egui::Color32::from_gray(64));

		assert_eq!(pixel_color(&analysis, 0), egui::Color32::GRAY);

		// The bad pixel of the active area is white:
		assert_eq!(pixel_color(&analysis, 5 * 10 + 5), egui::Color32::WHITE);

		// A good pixel of the active area is black:
		assert_eq!(pixel_color(&analysis, 5 * 10 + 4), egui::Color32::BLACK);
	}

	#[test]
	fn the_cluster_navigation_goes_through_the_relevant_clusters() {
		let mut app = App::default();

		// Three blocks of bad pixels, each of them a relevant cluster:
		let mut bad: Vec<(usize, usize)> = Vec::new();

		for (x, y) in [(5, 5), (15, 15), (25, 25)] {
			for dy in 0..3 {
				for dx in 0..3 {
					bad.push((x + dx, y + dy));
				}
			}
		}

		app.image = Some(image_with_bad(40, 40, &bad));

		app.analysis = Some(analyze(&app.image.as_ref().unwrap(), 0));

		assert_eq!(app.analysis.as_ref().unwrap().relevant_clusters.len(), 3);

		// The view of the image:
		app.display_rect =
			Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0)));

		app.view_rect =
			Some(egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(400.0, 400.0)));

		app.pixel_scale = 1.0;

		// Next shows the clusters one by one:
		app.next_cluster();

		assert_eq!(app.cluster_index, Some(0));

		app.next_cluster();

		assert_eq!(app.cluster_index, Some(1));

		app.next_cluster();

		assert_eq!(app.cluster_index, Some(2));

		// And it wraps around:
		app.next_cluster();

		assert_eq!(app.cluster_index, Some(0));

		// Previous goes the other way:
		app.prev_cluster();

		assert_eq!(app.cluster_index, Some(2));
	}

	#[test]
	fn the_arrow_keys_navigate_over_the_relevant_clusters() {
		let mut app = App::default();

		// Three blocks of bad pixels, each of them a relevant cluster:
		let mut bad: Vec<(usize, usize)> = Vec::new();

		for (x, y) in [(5, 5), (15, 15), (25, 25)] {
			for dy in 0..3 {
				for dx in 0..3 {
					bad.push((x + dx, y + dy));
				}
			}
		}

		app.image = Some(image_with_bad(40, 40, &bad));

		app.analysis = Some(analyze(&app.image.as_ref().unwrap(), 0));

		app.display_rect =
			Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0)));

		app.view_rect =
			Some(egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(400.0, 400.0)));

		app.pixel_scale = 1.0;

		let ctx = egui::Context::default();

		// The right arrow shows the next cluster:
		press_key(&ctx, &mut app, egui::Key::ArrowRight);

		assert_eq!(app.cluster_index, Some(0));

		press_key(&ctx, &mut app, egui::Key::ArrowRight);

		assert_eq!(app.cluster_index, Some(1));

		// The left arrow shows the previous one:
		press_key(&ctx, &mut app, egui::Key::ArrowLeft);

		assert_eq!(app.cluster_index, Some(0));

		// And it wraps around:
		press_key(&ctx, &mut app, egui::Key::ArrowLeft);

		assert_eq!(app.cluster_index, Some(2));
	}

	// Presses a key and lets the app react to it.
	fn press_key(ctx: &egui::Context, app: &mut App, key: egui::Key) {
		let mut input = egui::RawInput::default();

		input.events.push(egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		});

		run_frame(ctx, input, &mut |ui| app.navigate_clusters(ui.ctx()));
	}

	#[test]
	fn the_test_images_are_classified_as_their_names_say() {
		// The name of every test image holds the expected classification: the
		// single bad pixels, the line segments, the irrelevant clusters and the
		// relevant clusters.
		let images: &[(&str, usize, &[&str], &[&str], &[&str])] = &[
			(
				"1x24 line24 + rel3x3 cluster7-7.tif",
				0,
				&["1x24 Line24"],
				&[],
				&["rel3x3 cluster7-7"],
			),
			("1x32 line32.tif", 0, &["1x32 Line32"], &[], &[]),
			("1x32 line32C + 32x1 line32C.tif", 0, &["1x32 Line32C", "32x1 Line32C"], &[], &[]),
			(
				"24x1 line24 + rel3x3 cluster7-7.tif",
				0,
				&["24x1 Line24"],
				&[],
				&["rel3x3 cluster7-7"],
			),
			("24x2 line26.tif", 0, &["24x2 Line26"], &[], &[]),
			("2x2 cluster2.tif", 0, &[], &["2x2 cluster2"], &[]),
			("2x24 line26.tif", 0, &["2x24 Line26"], &[], &[]),
			("2x3 cluster4.tif", 0, &[], &["2x3 cluster4"], &[]),
			("32x1 line32.tif", 0, &["32x1 Line32"], &[], &[]),
			("3x2 cluster4.tif", 0, &[], &["3x2 cluster4"], &[]),
			("9 bad pixels.tif", 9, &[], &[], &[]),
			("rel3x4 cluster7-2.tif", 0, &[], &[], &["rel3x4 cluster7-2"]),
			("rel4x3 cluster7-2.tif", 0, &[], &[], &["rel4x3 cluster7-2"]),
			("single bad pixel.tif", 1, &[], &[], &[]),
		];

		for (name, single, lines, irrelevant, relevant) in images {
			let path = format!("{}/assets/images/{name}", env!("CARGO_MANIFEST_DIR"));

			let bytes = std::fs::read(&path).unwrap();

			let image = load_image(name, &bytes).unwrap();

			let analysis = analyze(&image, 0);

			assert_eq!(analysis.single_bad_pixels, *single, "the single bad pixels of {name}");

			let found: Vec<&str> =
				analysis.line_segments.iter().map(|defect| defect.name.as_str()).collect();

			assert_eq!(found, *lines, "the line segments of {name}");

			let found: Vec<&str> =
				analysis.irrelevant_clusters.iter().map(|defect| defect.name.as_str()).collect();

			assert_eq!(found, *irrelevant, "the irrelevant clusters of {name}");

			let found: Vec<&str> =
				analysis.relevant_clusters.iter().map(|defect| defect.name.as_str()).collect();

			assert_eq!(found, *relevant, "the relevant clusters of {name}");
		}
	}

	#[test]
	fn the_thumbnail_is_centered_in_the_report_panel() {
		// A tall image: its thumbnail is much narrower than the panel.
		let ctx = egui::Context::default();

		let mut app = App::default();

		app.set_image("tall.tif".to_owned(), image_with_bad(100, 400, &[(50, 200)]), &ctx);

		let mut input = egui::RawInput::default();

		input.screen_rect =
			Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0)));

		let mut rect = egui::Rect::NOTHING;

		run_frame(&ctx, input, &mut |ui| {
			egui::Panel::right(egui::Id::new("report")).exact_size(300.0).show(ui, |ui| {
				rect = app.show_thumbnail(ui);
			});
		});

		// The thumbnail is 200 points high and 50 points wide:
		assert_eq!(rect.height(), 200.0);

		assert_eq!(rect.width(), 50.0);

		// The panel is at the right of the screen:
		let panel =
			egui::Rect::from_min_max(egui::pos2(1000.0 - 300.0, 0.0), egui::pos2(1000.0, 800.0));

		// The thumbnail is centered in it:
		assert!((rect.center().x - panel.center().x).abs() < 1.0);
	}

	// Runs a frame with the given input.
	fn run_frame(
		ctx: &egui::Context,
		input: egui::RawInput,
		run: &mut dyn FnMut(&mut egui::Ui),
	) -> egui::FullOutput {
		let mut output = ctx.run_ui(input, |ui| run(ui));

		// The textures of the frame are applied, so that they are not dropped:
		let textures = ctx.tex_manager();

		for (id, deltas) in output.textures_delta.set.drain() {
			for delta in deltas {
				textures.write().set(id, delta);
			}
		}

		output.textures_delta.clear();

		output
	}

	#[cfg(not(target_arch = "wasm32"))]
	#[test]
	fn the_last_opened_image_is_remembered_in_the_registry() {
		use std::path::PathBuf;

		use winreg::RegKey;
		use winreg::enums::HKEY_CURRENT_USER;

		const KEY: &str = r"Software\ASTM";

		const VALUE: &str = "last_badpix_image";

		// The value of the key before the test:
		let previous = RegKey::predef(HKEY_CURRENT_USER)
			.open_subkey(KEY)
			.ok()
			.and_then(|key| key.get_value::<String, _>(VALUE).ok());

		// The opened image is remembered:
		let path = PathBuf::from(r"C:\Users\Test\Images\detector.tif");

		last_image::remember(&path);

		assert_eq!(last_image::path(), Some(path.clone()));

		// The file dialog opens in the directory of the image:
		assert_eq!(last_image::directory(), Some(PathBuf::from(r"C:\Users\Test\Images")));

		// The value of the key is restored:
		let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY).unwrap();

		match previous {
			Some(value) => key.set_value(VALUE, &value).unwrap(),
			None => {
				let _ = key.delete_value(VALUE);
			}
		}
	}

	#[test]
	fn the_pixels_of_the_crossing_of_two_lines_are_counted_once() {
		// The cross of a full row and a full column: 32 + 32 - 1 bad pixels.
		let path =
			format!("{}/assets/images/1x32 line32C + 32x1 line32C.tif", env!("CARGO_MANIFEST_DIR"));

		let bytes = std::fs::read(&path).unwrap();

		let image = load_image("cross.tif", &bytes).unwrap();

		let analysis = analyze(&image, 0);

		// The two lines cross at one pixel:
		assert_eq!(analysis.line_segments.len(), 2);

		let text = report_text(&image, &analysis);

		// The crossing pixel is counted once:
		assert!(text.contains("Total Defects = 63\n"));

		assert!(text.contains("Defects Area, % = 6,152344\n"));
	}

	#[test]
	fn a_cross_of_two_lines_is_two_line_segments() {
		// A horizontal and a vertical line that cross:
		let mut bad: Vec<(usize, usize)> = (0..20).map(|x| (x, 10)).collect();

		bad.extend((0..20).map(|y| (10, y)));

		let image = image_with_bad(20, 20, &bad);

		let analysis = analyze(&image, 0);

		// Both lines are found, not only the longest one:
		assert_eq!(analysis.line_segments.len(), 2);

		let mut names: Vec<&str> =
			analysis.line_segments.iter().map(|defect| defect.name.as_str()).collect();

		names.sort();

		assert_eq!(names, vec!["1x20 Line20C", "20x1 Line20C"]);

		// The crossing of the lines is not a cluster of its own: the crossing
		// pixels are the lines themselves.
		assert_eq!(analysis.relevant_clusters.len(), 0);

		// All the other bad pixels belong to the lines:
		assert_eq!(analysis.single_bad_pixels, 0);
		assert_eq!(analysis.irrelevant_clusters.len(), 0);
	}

	#[test]
	fn the_min_distance_between_the_parallel_lines_is_computed() {
		// Two horizontal lines, five rows apart, and one vertical line:
		let mut bad: Vec<(usize, usize)> = (0..20).map(|x| (x, 5)).collect();

		bad.extend((0..20).map(|x| (x, 10)));

		bad.extend((0..20).map(|y| (y, 15)));

		let image = image_with_bad(20, 20, &bad);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.line_segments.len(), 3);

		// The vertical line is not parallel to the horizontal ones, so it is
		// not compared with them:
		assert_eq!(analysis.min_line_distance, Some(5.0));

		// A single line has no distance to anything:
		let image = image_with_bad(20, 20, &[(0, 5), (1, 5), (2, 5)]);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.min_line_distance, None);
	}

	#[test]
	fn the_density_and_the_line_pixels_are_counted() {
		// A line of 12 pixels and a block of 5×5 pixels far from it:
		let mut bad: Vec<(usize, usize)> = (0..12).map(|x| (x, 2)).collect();

		for y in 12..17 {
			for x in 12..17 {
				bad.push((x, y));
			}
		}

		let image = image_with_bad(20, 20, &bad);

		let analysis = analyze(&image, 0);

		// The largest number of bad pixels in an area of 11×11 pixels:
		assert_eq!(analysis.max_density, 25);

		// The pixels of the line and the pixels outside of it:
		assert_eq!(analysis.line_pixels, 12);

		assert_eq!(analysis.non_line_pixels, 25);
	}

	#[test]
	fn the_image_can_not_be_panned_out_of_the_view() {
		let area = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));

		// The image is twice as large as the view:
		let fitted = egui::Rect::from_min_max(egui::pos2(-50.0, -50.0), egui::pos2(50.0, 50.0));

		let mut app = App::default();

		app.zoom = 2.0;

		// Pan the image far to the left and up:
		app.pan = egui::vec2(-10000.0, -10000.0);

		app.limit_pan(fitted, area);

		let display = egui::Rect::from_min_size(fitted.min + app.pan, fitted.size() * app.zoom);

		// A part of the image is still visible:
		assert!(display.max.x > area.min.x);
		assert!(display.max.y > area.min.y);

		// And the other way round:
		app.pan = egui::vec2(10000.0, 10000.0);

		app.limit_pan(fitted, area);

		let display = egui::Rect::from_min_size(fitted.min + app.pan, fitted.size() * app.zoom);

		assert!(display.min.x < area.max.x);
		assert!(display.min.y < area.max.y);
	}

	#[test]
	fn limit_pan_does_not_panic_when_the_image_fits_in_the_view() {
		// The view is larger than the image: min is larger than max, so the
		// former clamp panicked:
		let area = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(400.0, 400.0));

		let fitted = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));

		let mut app = App::default();

		app.zoom = 1.0;

		// Pan far away in both directions:
		app.pan = egui::vec2(-10000.0, -10000.0);

		// Must not panic:
		app.limit_pan(fitted, area);

		// And again with a large positive pan:
		app.pan = egui::vec2(10000.0, 10000.0);

		app.limit_pan(fitted, area);
	}

	#[test]
	fn limit_pan_does_not_panic_when_min_is_larger_than_max() {
		// With VISIBLE_MARGIN = 8, min is larger than max when
		// view_width + size.x < 16. A tiny view and a tiny image trigger it:
		let area = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(10.0, 10.0));

		let fitted = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(5.0, 5.0));

		let mut app = App::default();

		app.zoom = 1.0;

		app.pan = egui::vec2(0.0, 0.0);

		// Before the fix, clamp(min_x, max_x) panicked here because min_x (3)
		// is larger than max_x (2):
		app.limit_pan(fitted, area);
	}

	#[test]
	fn the_cluster_navigation_sets_the_caption() {
		let mut app = App::default();

		let mut bad: Vec<(usize, usize)> = Vec::new();

		for (x, y) in [(5, 5), (15, 15)] {
			for dy in 0..3 {
				for dx in 0..3 {
					bad.push((x + dx, y + dy));
				}
			}
		}

		app.image = Some(image_with_bad(40, 40, &bad));

		app.analysis = Some(analyze(&app.image.as_ref().unwrap(), 0));

		app.display_rect =
			Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0)));

		app.view_rect =
			Some(egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(400.0, 400.0)));

		app.pixel_scale = 1.0;

		assert!(app.cluster_caption.is_none());

		app.next_cluster();

		// The caption shows the number and the name of the cluster:
		let caption = app.cluster_caption.as_ref().unwrap();

		assert_eq!(caption.index, 0);
		assert_eq!(caption.total, 2);
		assert_eq!(caption.name, "rel3x3 cluster9-5");

		// The next cluster:
		app.next_cluster();

		assert_eq!(app.cluster_caption.as_ref().unwrap().index, 1);
	}

	#[test]
	fn a_dropped_file_is_opened() {
		use image::codecs::tiff::TiffEncoder;
		use image::{ExtendedColorType, ImageEncoder};

		// A TIFF with a single bad pixel:
		let (width, height) = (16u32, 16u32);

		let mut pixels = vec![0u16; (width * height) as usize];

		pixels[5 * width as usize + 5] = 100;

		let raw: Vec<u8> = pixels.iter().flat_map(|value| value.to_ne_bytes()).collect();

		let mut cursor = std::io::Cursor::new(Vec::new());

		TiffEncoder::new(&mut cursor)
			.write_image(&raw, width, height, ExtendedColorType::L16)
			.unwrap();

		let bytes = cursor.into_inner();

		let ctx = egui::Context::default();

		let mut app = App::default();

		// The file as the windowing system delivers it:
		#[derive(Debug)]
		struct Dropped {
			bytes: Vec<u8>,
		}

		impl egui::DroppedFile for Dropped {
			fn path(&self) -> &std::path::Path {
				std::path::Path::new("dropped.tif")
			}

			#[cfg(not(target_arch = "wasm32"))]
			fn bytes(&self) -> Result<Vec<u8>, String> {
				Ok(self.bytes.clone())
			}
		}

		let handle: egui::DroppedFileHandle = std::sync::Arc::new(Dropped { bytes });

		app.load_dropped(&ctx, vec![handle]);

		// The image is opened and analyzed:
		assert!(app.image.is_some());

		assert!(app.analysis.is_some());

		assert_eq!(app.analysis.as_ref().unwrap().single_bad_pixels, 1);

		assert!(app.status.contains("dropped.tif"));
	}

	#[test]
	fn the_bad_pixels_are_counted_by_their_box() {
		// A 10×10 image with a single bad pixel in the middle:
		let image = image_with_bad(10, 10, &[(5, 5)]);

		let analysis = analyze(&image, 0);

		// The bad pixel is alone in its 3×3 box, so it is Box 1. The good
		// pixels are not counted at all:
		assert_eq!(analysis.boxes[1], 1);

		for box_index in 2..10 {
			assert_eq!(analysis.boxes[box_index], 0);
		}

		// Every bad pixel of the active area is counted:
		assert_eq!(analysis.boxes.iter().sum::<usize>(), 1);

		// The pixels of the detector rim are not counted:
		let analysis = analyze(&image, 2);

		assert_eq!(analysis.boxes.iter().sum::<usize>(), 1);

		// A block of bad pixels: the corner pixels have four bad pixels in
		// their box, the ones on the edges six and the middle one nine:
		let bad: Vec<(usize, usize)> = (4..7).flat_map(|y| (4..7).map(move |x| (x, y))).collect();

		let image = image_with_bad(10, 10, &bad);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.boxes[4], 4);

		assert_eq!(analysis.boxes[6], 4);

		assert_eq!(analysis.boxes[9], 1);

		assert_eq!(analysis.boxes.iter().sum::<usize>(), 9);
	}

	#[test]
	fn a_cluster_at_the_side_of_a_line_is_separated_from_it() {
		// A line of 30 pixels with a 2×2 block at its side:
		let mut bad: Vec<(usize, usize)> = (0..30).map(|x| (x, 10)).collect();

		bad.push((12, 8));
		bad.push((13, 8));
		bad.push((12, 9));
		bad.push((13, 9));

		let image = image_with_bad(30, 20, &bad);

		let analysis = analyze(&image, 0);

		// The line is not split by the cluster that sits on it:
		assert_eq!(analysis.line_segments.len(), 1);
		assert_eq!(analysis.line_segments[0].name, "1x30 Line30");

		// The cluster is separated from the line:
		assert_eq!(analysis.relevant_clusters.len(), 1);
		assert_eq!(analysis.relevant_clusters[0].name, "rel3x2 cluster6-4");
	}

	#[test]
	fn reads_a_tiff_image() {
		use image::codecs::tiff::TiffEncoder;
		use image::{ExtendedColorType, ImageEncoder};

		let (width, height) = (16u32, 16u32);

		let mut pixels = vec![0u16; (width * height) as usize];

		pixels[5 * width as usize + 5] = 100;

		let raw: Vec<u8> = pixels.iter().flat_map(|value| value.to_ne_bytes()).collect();

		let mut cursor = std::io::Cursor::new(Vec::new());

		TiffEncoder::new(&mut cursor)
			.write_image(&raw, width, height, ExtendedColorType::L16)
			.unwrap();

		let image = load_raster(&cursor.into_inner()).unwrap();

		assert_eq!(image.width, 16);
		assert_eq!(image.height, 16);
		assert_eq!(image.pixels[5 * 16 + 5], 100);
		assert_eq!(analyze(&image, 0).single_bad_pixels, 1);
	}

	#[test]
	fn reads_a_diconde_image() {
		use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
		use dicom_object::{FileMetaTableBuilder, InMemDicomObject};

		let (width, height) = (16u16, 16u16);

		let mut pixels = vec![0u16; (width * height) as usize];

		pixels[5 * width as usize + 5] = 100;

		let mut object = InMemDicomObject::new_empty();

		object.put(DataElement::new(Tag(0x0028, 0x0010), VR::US, PrimitiveValue::from(height)));

		object.put(DataElement::new(Tag(0x0028, 0x0011), VR::US, PrimitiveValue::from(width)));

		object.put(DataElement::new(Tag(0x0028, 0x0002), VR::US, PrimitiveValue::from(1u16)));

		object.put(DataElement::new(
			Tag(0x0028, 0x0004),
			VR::CS,
			PrimitiveValue::from("MONOCHROME2"),
		));

		object.put(DataElement::new(Tag(0x0028, 0x0100), VR::US, PrimitiveValue::from(16u16)));

		object.put(DataElement::new(Tag(0x0028, 0x0101), VR::US, PrimitiveValue::from(16u16)));

		object.put(DataElement::new(Tag(0x0028, 0x0102), VR::US, PrimitiveValue::from(15u16)));

		object.put(DataElement::new(Tag(0x0028, 0x0103), VR::US, PrimitiveValue::from(0u16)));

		object.put(DataElement::new(
			Tag(0x7FE0, 0x0010),
			VR::OW,
			PrimitiveValue::U16(pixels.clone().into()),
		));

		let file_object = object
			.with_meta(FileMetaTableBuilder::new().transfer_syntax("1.2.840.10008.1.2.1"))
			.unwrap();

		let mut cursor = std::io::Cursor::new(Vec::new());

		file_object.write_all(&mut cursor).unwrap();

		let image = load_dicom(&cursor.into_inner()).unwrap();

		assert_eq!(image.width, 16);
		assert_eq!(image.height, 16);
		assert_eq!(image.pixels[5 * 16 + 5], 100);
		assert_eq!(analyze(&image, 0).single_bad_pixels, 1);
	}

	#[test]
	fn rim_pixels_need_fewer_good_neighbours() {
		// A bad pixel on the detector rim edge with three good neighbours is
		// not a cluster kernel pixel...
		let image = image_with_bad(10, 10, &[(0, 5), (1, 5)]);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.relevant_clusters.len(), 0);
		assert_eq!(analysis.irrelevant_clusters.len(), 1);

		// ... but a bad pixel in the rim corner with less than two good
		// neighbours is one.
		let image = image_with_bad(10, 10, &[(0, 0), (1, 0), (0, 1)]);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.relevant_clusters.len(), 1);
		assert_eq!(analysis.relevant_clusters[0].name, "rel2x2 cluster3-1");
	}

	#[test]
	fn the_detector_rim_is_not_evaluated() {
		// A bad pixel in the rim and a bad pixel in the active area:
		let image = image_with_bad(10, 10, &[(1, 1), (5, 5)]);

		let analysis = analyze(&image, 2);

		// The bad pixel in the rim is cut off, the other one is a single bad pixel:
		assert_eq!(analysis.single_bad_pixels, 1);
		assert_eq!(analysis.relevant_clusters.len(), 0);
		assert_eq!(analysis.irrelevant_clusters.len(), 0);

		// The rim is the border of the image:
		assert!(analysis.rim[0]);
		assert!(analysis.rim[1 * 10 + 1]);
		assert!(!analysis.rim[5 * 10 + 5]);
		assert_eq!(analysis.rim_width, 2);
	}

	#[test]
	fn the_detector_rim_is_the_edge_of_the_detector() {
		// A 2×2 block at the edge of the active area. Without a rim every pixel
		// of the block still has five good neighbours, so the cluster is
		// irrelevant. With a rim of two pixels the block is at the edge of the
		// detector and its pixels have less than three good neighbours, so they
		// become cluster kernel pixels.
		let bad = [(2, 5), (3, 5), (2, 6), (3, 6)];

		let image = image_with_bad(10, 10, &bad);

		let analysis = analyze(&image, 0);

		assert_eq!(analysis.relevant_clusters.len(), 0);
		assert_eq!(analysis.irrelevant_clusters.len(), 1);

		let analysis = analyze(&image, 2);

		assert_eq!(analysis.irrelevant_clusters.len(), 0);
		assert_eq!(analysis.relevant_clusters.len(), 1);
		assert_eq!(analysis.relevant_clusters[0].name, "rel2x2 cluster4-2");
	}
}

// desktop version
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
	let options = eframe::NativeOptions {
		viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 760.0]),
		..Default::default()
	};

	eframe::run_native(
		"Bad Pixel Analyzer",
		options,
		Box::new(|cc| {
			// Use a light theme regardless of the OS theme (set on both styles, since the
			// theme preference decides which one is active at runtime).
			cc.egui_ctx.set_theme(egui::Theme::Light);
			cc.egui_ctx.set_visuals_of(egui::Theme::Dark, egui::Visuals::light());
			cc.egui_ctx.set_visuals_of(egui::Theme::Light, egui::Visuals::light());
			egui_extras::install_image_loaders(&cc.egui_ctx);
			Ok(Box::new(App::default()))
		}),
	)
}

// web version
#[cfg(target_arch = "wasm32")]
fn main() {
	use eframe::wasm_bindgen::JsCast as _;

	let web_options = eframe::WebOptions::default();

	wasm_bindgen_futures::spawn_local(async {
		let document = web_sys::window().expect("No window").document().expect("No document");

		let canvas = document
			.get_element_by_id("the_canvas_id")
			.expect("Failed to find the_canvas_id")
			.dyn_into::<web_sys::HtmlCanvasElement>()
			.expect("the_canvas_id was not a HtmlCanvasElement");

		let start_result = eframe::WebRunner::new()
			.start(
				canvas,
				web_options,
				Box::new(|cc| {
					// Use a light theme regardless of the OS theme (set on both styles, since
					// the theme preference decides which one is active at runtime).
					cc.egui_ctx.set_theme(egui::Theme::Light);
					cc.egui_ctx.set_visuals_of(egui::Theme::Dark, egui::Visuals::light());
					cc.egui_ctx.set_visuals_of(egui::Theme::Light, egui::Visuals::light());
					egui_extras::install_image_loaders(&cc.egui_ctx);
					Ok(Box::new(App::default()))
				}),
			)
			.await;

		let loading_text = web_sys::window()
			.and_then(|window| window.document())
			.and_then(|document| document.get_element_by_id("loading_text"));

		if let Some(loading_text) = loading_text {
			match start_result {
				Ok(_) => loading_text.remove(),
				Err(e) => {
					loading_text.set_inner_html("<p> The app crashed (see console). </p>");
					panic!("Failed to start eframe: {e:?}");
				}
			}
		}
	});
}
