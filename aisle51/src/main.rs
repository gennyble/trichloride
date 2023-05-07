use std::fs::File;

use devout::{Devout, Framerate};

fn main() {
	let mut frout = Frameout::new(1280, 720);
	let blackyuv = rgb_yuv(255, 0, 0);
	println!("{blackyuv:?}");
	//return;
	frout.rect(0, 0, 1280, 720, rgb_yuv(0, 0, 0));

	let file = File::create("out.mp4").unwrap();
	let mut dev = Devout::new(file, Framerate::Thirty);

	for tick in 0..=255u8 {
		println!("{tick}");
		let yuv = rgb_yuv(tick / 2, 0, tick);
		frout.rect(20 + tick as usize * 3, 330, 60, 60, yuv);
		dev.frame_yuv420(1280, 720, &frout.buffer);
	}

	dev.done();
}

// Stores YUV420 and manages the drawing so we only draw what's changed
struct Frameout {
	width: usize,
	height: usize,
	buffer: Vec<u8>,

	/// Keeps track of frame positioning and animation things.
	tick: usize,
}

impl Frameout {
	pub fn new(width: usize, height: usize) -> Self {
		Frameout {
			width,
			height,
			buffer: vec![0; width * height + (width * height) / 2],
			tick: 0,
		}
	}

	pub fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, yuv: Yuv<u8>) {
		let yspan = self.width * self.height;
		//TODO: gen- check bounds
		for yidx in 0..h {
			let y = y + yidx;
			for xidx in 0..w {
				let x = x + xidx;

				let y_idx = y * self.width + x;
				let subsample_idx = (self.width / 2) * (y / 2) + (x / 2);
				let u_idx = yspan + subsample_idx;
				let v_idx = yspan + yspan / 4 + subsample_idx;

				self.buffer[y_idx] = yuv.y;
				self.buffer[u_idx] = yuv.u;
				self.buffer[v_idx] = yuv.v;
			}
		}
	}
}

// https://en.wikipedia.org/wiki/YUV#Y%E2%80%B2UV444_to_RGB888_conversion
fn rgb_yuv(r: u8, g: u8, b: u8) -> Yuv<u8> {
	let r = r as f32 / 255.0;
	let g = g as f32 / 255.0;
	let b = b as f32 / 255.0;

	Yuv {
		y: 0.299 * r + 0.587 * g + 0.114 * b,
		u: -0.147 * r - 0.289 * g + 0.436 * b + 0.5,
		v: 0.615 * r - 0.515 * g - 0.100 * b + 0.5,
	}
	.tou8()
}

#[derive(Debug)]
struct Yuv<T> {
	y: T,
	u: T,
	v: T,
}

impl Yuv<f32> {
	pub fn tou8(self) -> Yuv<u8> {
		Yuv {
			y: (self.y * 255.0).clamp(0.0, 255.0) as u8,
			u: (self.u * 255.0).clamp(0.0, 255.0) as u8,
			v: (self.v * 255.0).clamp(0.0, 255.0) as u8,
		}
	}
}
