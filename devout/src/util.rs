use openh264::formats::YUVSource;

#[derive(Copy, Clone, Debug)]
pub enum Framerate {
	/// That weird 29.97 FPS of NTSC.
	///
	/// Here's a video by Stand-up Maths about that if you were curious like, why its like that.  
	/// <https://www.youtube.com/watch?v=3GJUM6pCpew>
	NTSC,
	/// 25 FPS
	PAL,
	/// 24 FPS. Commonly used in movies.
	TwentyFour,
	/// 30 FPS
	Thirty,
	/// 60 FPS
	Sixty,
	/// Whole numbered FPS. This number * 1000 is the timescale.
	Whole(u32),
	/// For those weird, niche uses where you *really* want to have the most
	/// control over a crate that by design does not give you much control.
	Custom {
		ticks_per_frame: u32,
		timescale: u32,
	},
}

impl Framerate {
	pub fn tpf(&self) -> u32 {
		match self {
			Framerate::NTSC => 100,
			Framerate::PAL => 1000,
			Framerate::TwentyFour => 1000,
			Framerate::Thirty => 512,
			Framerate::Sixty => 256,
			Framerate::Whole(_) => 1000,
			Framerate::Custom {
				ticks_per_frame, ..
			} => *ticks_per_frame,
		}
	}

	pub fn timescale(&self) -> u32 {
		match self {
			Framerate::NTSC => 2997,
			Framerate::PAL => 25000,
			Framerate::TwentyFour => 24000,
			// FFMPEG uses this and I think it's cute
			Framerate::Thirty => 15360,
			Framerate::Sixty => 15360,
			Framerate::Whole(w) => *w * 1000,
			Framerate::Custom { timescale, .. } => *timescale,
		}
	}
}

impl From<u8> for Framerate {
	fn from(n: u8) -> Framerate {
		Framerate::Whole(n as u32)
	}
}

impl From<u16> for Framerate {
	fn from(n: u16) -> Framerate {
		Framerate::Whole(n as u32)
	}
}

impl From<u32> for Framerate {
	fn from(n: u32) -> Framerate {
		Framerate::Whole(n)
	}
}

/// YUV420 planar struct *(also known as YUV420p)* organized so that all
/// Y data appears, then all U, and then all V.
pub(crate) struct YUV420Wrapper<'a> {
	pub width: usize,
	pub height: usize,
	pub bytes: &'a [u8],
}

// Based off https://docs.rs/openh264/latest/src/openh264/formats/rgb2yuv.rs.html#4-8
impl<'a> YUVSource for YUV420Wrapper<'a> {
	fn width(&self) -> i32 {
		self.width as i32
	}

	fn height(&self) -> i32 {
		self.height as i32
	}

	fn y(&self) -> &[u8] {
		&self.bytes[..self.width * self.height]
	}

	fn u(&self) -> &[u8] {
		let base_u = self.width * self.height;
		// This looked weird to me in the openh264-rust crate, but I understand it now.
		// The end part of the range is there because we *start* there, and then we add
		// the length of the U channel, which is a quarter of the width * height. But
		// `base_u` is already that, so we use it again and divide by 4.
		&self.bytes[base_u..base_u + base_u / 4]
	}

	fn v(&self) -> &[u8] {
		let base_u = self.width * self.height;
		let base_v = base_u + base_u / 4;
		&self.bytes[base_v..]
	}

	fn y_stride(&self) -> i32 {
		self.width as i32
	}

	fn u_stride(&self) -> i32 {
		(self.width / 2) as i32
	}

	fn v_stride(&self) -> i32 {
		(self.width / 2) as i32
	}
}
