use std::{borrow::BorrowMut, num::NonZeroU32, rc::Rc};

use softbuffer::{Context, Surface};
use winit::{
	event::{Event, WindowEvent},
	event_loop::{ControlFlow, EventLoop, EventLoopWindowTarget},
	window::Window,
};

pub use winit::dpi::{LogicalSize, PhysicalSize};
pub use winit::event;
pub use winit::event_loop;
pub use winit::window::WindowBuilder;

pub struct FluffyWindow {
	// `run` makes me so tired. i want `poll_events` so bad. the "Caveats" on `run_return` scare me
	pub event_loop: Option<EventLoop<()>>,
	pub window: Rc<Window>,

	pub context: Context<Rc<Window>>,
	pub surface: Surface<Rc<Window>, Rc<Window>>,
}

impl FluffyWindow {
	pub fn build_window(window_builder: WindowBuilder) -> Self {
		let event_loop = EventLoop::new().unwrap();
		let window = Rc::new(window_builder.build(&event_loop).unwrap());
		let context = unsafe { softbuffer::Context::new(window.clone()) }.unwrap();
		let surface = unsafe { softbuffer::Surface::new(&context, window.clone()) }.unwrap();

		let mut this = Self {
			event_loop: Some(event_loop),
			window,
			context,
			surface,
		};
		this.resize_surface(this.window.inner_size());
		this
	}

	/*pub fn resize(&mut self, width: usize, height: usize) {
		let phys = PhysicalSize::new(width as u32, height as u32);
		self.window.borrow_mut().set_max_inner_size(max_size)
		//self.resize_surface(phys)
	}*/

	fn resize_surface(&mut self, PhysicalSize { width, height }: PhysicalSize<u32>) {
		let width = NonZeroU32::new(width.max(1)).unwrap();
		let height = NonZeroU32::new(height.max(1)).unwrap();
		self.surface.resize(width, height).unwrap();
	}

	pub fn buffer(&mut self) -> Buffer<'_> {
		let winsize = self.window.inner_size();
		Buffer::new(
			self.surface.buffer_mut().unwrap(),
			winsize.width as usize,
			winsize.height as usize,
		)
	}

	pub fn common_events(&mut self, event: &Event<()>, el: &EventLoopWindowTarget<()>) {
		match event {
			Event::WindowEvent {
				event: WindowEvent::Resized(phys),
				..
			} => {
				self.resize_surface(*phys);
				self.window.request_redraw();
			}

			Event::WindowEvent {
				event: WindowEvent::CloseRequested,
				..
			} => el.exit(),

			_ => (),
		}
	}

	/// Take the event loop from Fluffy, leaving `None` it it's place. This is
	/// neccesary 'cause lifetimes and ownership ahhhh. If there's no loop it panics
	pub fn take_el(&mut self) -> EventLoop<()> {
		self.event_loop.take().unwrap()
	}
}

pub struct Buffer<'a> {
	/// Bytes - 0RGB
	pub data: softbuffer::Buffer<'a, Rc<Window>, Rc<Window>>,
	pub width: usize,
	pub height: usize,
}

impl<'a> Buffer<'a> {
	pub fn new(
		data: softbuffer::Buffer<'a, Rc<Window>, Rc<Window>>,
		width: usize,
		height: usize,
	) -> Self {
		Buffer {
			data,
			width,
			height,
		}
	}

	//TODO: gen- Check it's the right size and like, rename it probably, too
	pub fn as_rgb_bytes(&self, rgb: &mut [u8]) {
		for (idx, px) in self.data.iter().enumerate() {
			let bytes = px.to_be_bytes();
			// bytes[0] is the 0x00 of 00RRGGBB
			rgb[idx * 3] = bytes[1];
			rgb[idx * 3 + 1] = bytes[2];
			rgb[idx * 3 + 2] = bytes[3];
		}
	}

	pub fn clear(&mut self) {
		self.data.fill(0)
	}

	/// Yeah. That's what I'm calling this, really.
	///
	/// Takes a (f64, f64) tuple in the range [0,1000] and rescales it to fall
	/// within the width and height so you can get (x,y)
	pub fn dethou(&self, tup: (f64, f64)) -> (usize, usize) {
		(
			(tup.0 * (self.width - 1) as f64 / 1000.0).floor() as usize,
			(tup.1 * (self.height - 1) as f64 / 1000.0).floor() as usize,
		)
	}

	/// Set a pixel with the RGB value
	pub fn set(&mut self, x: usize, y: usize, c: Color) {
		if y >= self.height || x >= self.width {
			return;
		}

		self.set_unchecked(x, y, c)
	}

	pub fn set_unchecked(&mut self, x: usize, y: usize, c: Color) {
		if let Some(px) = self.data.get_mut(y * self.width + x) {
			*px = c.u32();
		}
		//let px = &mut self.data[y * self.width + x];
	}

	pub fn rect(&mut self, x: usize, y: usize, width: usize, height: usize, c: Color) {
		//TODO: check x and y are in range before we loop so we don't check every time
		for px in x..x + width {
			for py in y..y + height {
				self.set(px, py, c)
			}
		}
	}

	/// Draw a vertical line :D
	/// Range is [y_start,y_end). I.E. start is incldued, end is not. If start
	/// is greater than end, the two are swapped.
	pub fn vert(&mut self, x: usize, y_start: usize, y_end: usize, c: Color) {
		let ymin = y_start.min(y_end);
		let ymax = y_start.max(y_end).clamp(0, self.height);

		for y in ymin..ymax {
			self.set_unchecked(x, y, c);
		}
	}

	/// Draw a horizontal line :D
	/// Range is [x_start,x_end). I.E. start is included, end is not. If start
	/// is greater than end, the two are swapped
	pub fn hori(&mut self, y: usize, x_start: usize, x_end: usize, c: Color) {
		let xmin = x_start.min(x_end);
		let xmax = x_start.max(x_end).clamp(0, self.width);

		for x in xmin..xmax {
			self.set_unchecked(x, y, c);
		}
	}

	pub fn image(&mut self, x: usize, y: usize, image: &PaletteImage) {
		for dy in y..y + image.height {
			for dx in x..x + image.width {
				let idx = (dy - y) * image.width + (dx - x);

				if let Some(color_idx) = image.data.get(idx) {
					match (
						image.palette.get(*color_idx as usize),
						image.trns.get(*color_idx as usize),
					) {
						(_, Some(0)) => (),
						(Some(color), _) => {
							self.set(dx, dy, *color);
						}
						_ => (),
					}
				}
			}
		}
	}

	pub fn isize_image(&mut self, x: isize, y: isize, image: &PaletteImage) {
		let y_neg = y.min(0).abs() as usize;
		let x_neg = x.min(0).abs() as usize;

		let y_start = y.max(0) as usize;
		let x_start = x.max(0) as usize;

		let image_h_run = image.height - y_neg;
		let image_w_run = image.width - x_neg;

		for dy in y_start..y_start + image_h_run {
			for dx in x_start..x_start + image_w_run {
				let idx = (dy - y_start + y_neg) * image.width + (dx - x_start + x_neg);

				if let Some(color_idx) = image.data.get(idx) {
					match (
						image.palette.get(*color_idx as usize),
						image.trns.get(*color_idx as usize),
					) {
						(_, Some(0)) => (),
						(Some(color), _) => {
							self.set(dx, dy, *color);
						}
						_ => (),
					}
				}
			}
		}
	}

	pub fn present(self) {
		self.data.present().unwrap()
	}
}

pub struct PaletteImage {
	pub width: usize,
	pub height: usize,
	pub data: Vec<u8>,
	pub palette: Vec<Color>,
	pub trns: Vec<u8>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Color {
	pub r: u8,
	pub g: u8,
	pub b: u8,
}

impl Color {
	pub const WHITE: Color = Color::new(0xFF, 0xFF, 0xFF);
	pub const GENTLE_LILAC: Color = Color::new(0xDD, 0xAA, 0xFF);
	pub const EMU_TURQUOISE: Color = Color::new(0x33, 0xAA, 0x88);
	pub const GREY_DD: Color = Color::new(0xDD, 0xDD, 0xDD);
	pub const GREY_44: Color = Color::new(0x44, 0x44, 0x44);

	pub const fn new(r: u8, g: u8, b: u8) -> Self {
		Color { r, g, b }
	}

	pub const fn u32(&self) -> u32 {
		u32::from_be_bytes([0, self.r, self.g, self.b])
	}
}
