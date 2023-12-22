use crate::capture::{BorrowedFrame, Frame};

pub trait Vex {
	fn frame_in(&mut self, frame: BorrowedFrame);
	fn frame_out(&mut self) -> BorrowedFrame;

	fn effect(&mut self, frame: BorrowedFrame) -> BorrowedFrame {
		self.frame_in(frame);
		self.frame_out()
	}

	/// Takes ownership of the effect and returns a frame to use to prime
	/// the next effect
	fn into_frame(self) -> Frame;
}

pub struct Tricrideo {
	buffer: Frame,
	channel_idx: u8,
	coloured: bool,
}

impl Tricrideo {
	pub fn new(width: usize, height: usize) -> Self {
		Self {
			buffer: Frame {
				data: vec![0; width * height * 3],
				width,
				height,
			},
			channel_idx: 0,
			coloured: false,
		}
	}

	pub fn from_frame(frame: Frame) -> Self {
		Self {
			buffer: frame,
			channel_idx: 0,
			coloured: false,
		}
	}

	pub fn set_coloured(&mut self, coloured: bool) {
		self.coloured = coloured;
	}

	fn gray(&mut self, frame: BorrowedFrame) {
		let rgb = frame.data;

		for (idx, px) in self.buffer.data.chunks_mut(3).enumerate() {
			// Do a naïve average of the incoming pixels colour
			let new = ((rgb[idx * 3] as u32 + rgb[idx * 3 + 1] as u32 + rgb[idx * 3 + 2] as u32)
				/ 3) as u8;

			// Shift colours one forward (R -> G, G -> B) and set out new red
			px[1] = px[0];
			px[2] = px[1];
			px[0] = new;
		}
	}

	fn colour(&mut self, frame: BorrowedFrame) {
		for (idx, px) in self.buffer.data.chunks_mut(3).enumerate() {
			px[self.channel_idx as usize] = frame.data[idx * 3 + self.channel_idx as usize];
		}
	}
}

impl Vex for Tricrideo {
	fn into_frame(self) -> Frame {
		self.buffer
	}

	fn frame_in(&mut self, frame: BorrowedFrame) {
		if self.coloured {
			self.colour(frame)
		} else {
			self.gray(frame)
		}

		self.channel_idx = (self.channel_idx + 1) % 3;
	}

	fn frame_out(&mut self) -> BorrowedFrame {
		BorrowedFrame {
			data: &self.buffer.data,
			width: self.buffer.width,
			height: self.buffer.height,
		}
	}
}
