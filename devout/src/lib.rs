use std::io::{Seek, Write};

use bytes::BytesMut;
use mp4::{AvcConfig, Bytes, MediaConfig, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig};
use openh264::{
	encoder::{EncodedBitStream, Encoder, EncoderConfig},
	formats::{YUVBuffer, YUVSource},
};

//TODO: gen- Do we want to mp4.done() on drop???

struct WriterWrapper<W: Write + Seek> {
	writer: Option<W>,
	mp4_writer: Option<Mp4Writer<W>>,
}

impl<W: Write + Seek> WriterWrapper<W> {
	pub fn new(writer: W) -> Self {
		Self {
			writer: Some(writer),
			mp4_writer: None,
		}
	}

	pub fn mp4_or_create_with<F>(&mut self, f: F) -> &mut Mp4Writer<W>
	where
		F: FnOnce(W) -> Mp4Writer<W>,
	{
		if self.mp4_writer.is_some() {
			//TODO : gen- not this
			self.mp4_writer.as_mut().unwrap()
		} else {
			// TODO: gen-
			// WriterWrapper should not exist in a state without the writer or mp4_writer,
			// but we should maybe not unwrap here, still
			let writer = self.writer.take().unwrap();
			let init = || f(writer);
			self.mp4_writer.get_or_insert_with(init)
		}
	}
}

pub struct Devout<W: Write + Seek> {
	framerate: Framerate,
	encoder: Option<Maybeh264>,
	writer: WriterWrapper<W>,
	sample_buffer: BytesMut,
	ticks: u64,
}

/// The things we need to encode H264.
struct Maybeh264 {
	encoder: Encoder,
	yuvbuffer: YUVBuffer,
}

impl<W: Write + Seek> Devout<W> {
	/// Get a new instance. All this does is form the struct. All initialization
	/// of the H264 encoder and MP4 writer is done on the first frame. If you
	/// want to create the H264 encoder at the same time, use new_with_dimensions
	//TODO: gen- Link to referred method above
	pub fn new<R: Into<Framerate>>(writer: W, framerate: R) -> Self {
		Self {
			framerate: framerate.into(),
			encoder: None,
			writer: WriterWrapper::new(writer),
			sample_buffer: BytesMut::new(),
			ticks: 0,
		}
	}

	/// Get a new ['Devout'] and create the internal H264 encoder at the same
	/// time.
	///
	/// Note, we still create the MP4 writer upoin receiving the first
	/// frame as there is information we can only gather after feeding OpenH264
	/// at least one frame.
	pub fn new_with_dimensions<R: Into<Framerate>>(
		writer: W,
		framerate: R,
		width: u32,
		height: u32,
	) -> Self {
		Self {
			framerate: framerate.into(),
			encoder: Some(Self::init_encoder(width, height)),
			writer: WriterWrapper::new(writer),
			sample_buffer: BytesMut::new(),
			ticks: 0,
		}
	}

	fn init_encoder(width: u32, height: u32) -> Maybeh264 {
		let encoder =
			Encoder::with_config(EncoderConfig::new(width as u32, height as u32)).unwrap();
		let yuvbuffer = YUVBuffer::new(width as usize, height as usize);

		Maybeh264 { encoder, yuvbuffer }
	}

	/// To be called when you're done writing data. Writes the last of the MP4.
	pub fn done(self) {
		let mut mp4 = self.writer.mp4_writer.unwrap();
		mp4.write_end().unwrap();
	}

	/// Take a frame, as 24bit RGB, and push it through into the video. If
	pub fn frame(&mut self, width: u32, height: u32, data: &[u8]) {
		/* TODO: gen- Write this, lol */
		#[rustfmt::skip]
		let encoder = self.encoder.get_or_insert_with(|| Self::init_encoder(width, height));
		encoder.yuvbuffer.read_rgb(data);
		self.write_frame::<YUV420Wrapper>(width, height, None)
	}

	/// Take a frame already encoded as YU 4:2:0 and push it to the video stream
	pub fn frame_yuv420(&mut self, width: u32, height: u32, data: &[u8]) {
		//TODO: check width/height is correct with known width/height AND with data given
		self.write_frame(
			width,
			height,
			Some(&YUV420Wrapper {
				width: width as usize,
				height: height as usize,
				bytes: data,
			}),
		)
	}

	fn write_frame<Y: YUVSource>(&mut self, width: u32, height: u32, yuv: Option<&Y>) {
		#[rustfmt::skip]
		let encoder = self.encoder.get_or_insert_with(|| Self::init_encoder(width, height));
		let bitstream = match yuv {
			Some(yuv) => encoder.encoder.encode(yuv).unwrap(),
			None => encoder.encoder.encode(&encoder.yuvbuffer).unwrap(),
		};

		let mp4_init_closure = |writer: W| {
			Self::init_mp4(
				&bitstream,
				writer,
				&self.framerate,
				width as u16,
				height as u16,
			)
		};

		let mp4_writer = self.writer.mp4_or_create_with(mp4_init_closure);
		let bytes = Self::fill_sample_buffer(&mut self.sample_buffer, &bitstream);

		let duration = self.framerate.tpf();
		let sample = Mp4Sample {
			start_time: self.ticks,
			duration,
			rendering_offset: 0,
			is_sync: false,
			bytes,
		};
		self.ticks += duration as u64;

		mp4_writer.write_sample(1, &sample).unwrap();
	}

	fn init_mp4(
		bitstream: &EncodedBitStream,
		writer: W,
		framerate: &Framerate,
		width: u16,
		height: u16,
	) -> Mp4Writer<W> {
		let mut sps = None;
		let mut pps = None;

		'layers: for layer_idx in 0..bitstream.num_layers() {
			let layer = bitstream.layer(layer_idx).unwrap();
			for nal_idx in 0..layer.nal_count() {
				let nal = layer.nal_unit(nal_idx).unwrap();

				let nal_data = Self::nal_data(nal);

				if sps.is_none() && nal_data[0] & 0x1F == 7 {
					sps = Some(nal_data.to_vec());
				}

				if pps.is_none() && nal_data[0] & 0x1F == 8 {
					pps = Some(nal_data.to_vec());
				}

				// We found them!!! :D
				if sps.is_some() && pps.is_some() {
					break 'layers;
				}
			}
		}

		let config = Mp4Config {
			major_brand: "isom".parse().unwrap(),
			minor_version: 512,
			compatible_brands: vec![
				str::parse("isom").unwrap(),
				str::parse("iso2").unwrap(),
				str::parse("avc1").unwrap(),
				str::parse("mp41").unwrap(),
			],
			timescale: 1000,
		};

		let mut mp4_writer = Mp4Writer::write_start(writer, &config).unwrap();

		let track_config = TrackConfig {
			track_type: mp4::TrackType::Video,
			timescale: framerate.timescale(),
			language: String::from("und"),
			media_conf: MediaConfig::AvcConfig(AvcConfig {
				width,
				height,
				seq_param_set: sps.unwrap(),
				pic_param_set: pps.unwrap(),
			}),
		};

		mp4_writer.add_track(&track_config).unwrap();
		mp4_writer
	}

	/// skip the 001 or 0001 of a nal to get to the data. If the nal doesn't
	/// start with either preamble, the slice is returned unchanged.
	#[inline]
	fn nal_data(nal: &[u8]) -> &[u8] {
		match nal {
			[0, 0, 1, ..] => &nal[3..],
			[0, 0, 0, 1, ..] => &nal[4..],
			// uhHHhHHhH skip data that doesn't look like a nal but was??
			_ => nal,
		}
	}

	#[inline]
	fn fill_sample_buffer<'a>(buffer: &'a mut BytesMut, bitstream: &EncodedBitStream) -> Bytes {
		buffer.clear();

		for layer_idx in 0..bitstream.num_layers() {
			let layer = bitstream.layer(layer_idx).unwrap();
			for nal_idx in 0..layer.nal_count() {
				let nal = layer.nal_unit(nal_idx).unwrap();

				let nal_data = Self::nal_data(nal);
				let length = nal_data.len() as u32;

				// We don't want/need to write out Sequence Parameter Sets
				if nal_data[0] & 0x1F != 7 {
					buffer.extend_from_slice(&length.to_be_bytes());
					buffer.extend_from_slice(nal_data);
				}
			}
		}

		buffer.split().freeze()
	}
}

//TODO: gen- Make use of this, lol. Just wanted to draft it. For non whole
// framrates *(looking at you 29.97)* that aren't declared here, I think we
// should take a numerator and a denomenator. Maybe we can make them from a
// float? how do the calculators and stuff get the denom and numerator from
// a decimal. do they just try numbers until it works? do I need a lookup table?
// I do not want to make a lookup table.
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
	// Oh no.
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

impl Into<Framerate> for u8 {
	fn into(self) -> Framerate {
		Framerate::Whole(self as u32)
	}
}

impl Into<Framerate> for u16 {
	fn into(self) -> Framerate {
		Framerate::Whole(self as u32)
	}
}

impl Into<Framerate> for u32 {
	fn into(self) -> Framerate {
		Framerate::Whole(self)
	}
}

struct YUV420Wrapper<'a> {
	width: usize,
	height: usize,
	bytes: &'a [u8],
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
