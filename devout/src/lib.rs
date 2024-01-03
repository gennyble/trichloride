use core::fmt;
use std::{
	fs::File,
	io::{BufWriter, Seek, Write},
	path::Path,
};

use bytes::BytesMut;
use mp4::{AvcConfig, Bytes, MediaConfig, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig};
use openh264::{
	encoder::{EncodedBitStream, Encoder, EncoderConfig},
	formats::YUVBuffer,
};

#[rustfmt::skip]
/*pub*/ use openh264::formats::YUVSource;

pub use util::Framerate;
use util::YUV420Wrapper;

mod util;

struct WriterWrapper<W: Write + Seek> {
	writer: Option<W>,
	mp4_writer: Option<Mp4Writer<W>>,
}

impl<W: Write + Seek> WriterWrapper<W> {
	fn new(writer: W) -> Self {
		Self {
			writer: Some(writer),
			mp4_writer: None,
		}
	}

	fn mp4_or_create_with<F>(&mut self, f: F) -> &mut Mp4Writer<W>
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

impl<W: Write + Seek> Drop for WriterWrapper<W> {
	fn drop(&mut self) {
		if let Some(ref mut writer) = self.mp4_writer {
			writer.write_end().unwrap()
		}
	}
}

pub struct Devout<W: Write + Seek> {
	framerate: Framerate,
	bitrate_kbps: u32,
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

impl Devout<BufWriter<File>> {
	/// Get a new [Devout] that's writing to a buffered file.
	pub fn file<P: AsRef<Path>, R: Into<Framerate>>(
		path: P,
		framerate: R,
	) -> Result<Self, std::io::Error> {
		Ok(Self::new(BufWriter::new(File::create(path)?), framerate))
	}
}

impl<W: Write + Seek> Devout<W> {
	/// Get a new instance.
	///
	/// All this does is form the struct. All initialization
	/// of the H264 encoder and MP4 writer is done on the first frame. If you
	/// want to create the H264 encoder at the same time, use [Devout::new_with_dimensions()]
	pub fn new<R: Into<Framerate>>(writer: W, framerate: R) -> Self {
		Self {
			framerate: framerate.into(),
			bitrate_kbps: 1000,
			encoder: None,
			writer: WriterWrapper::new(writer),
			sample_buffer: BytesMut::new(),
			ticks: 0,
		}
	}

	/// Get a new [Devout] and create the internal H264 encoder at the same
	/// time.
	///
	/// Note, we still create the MP4 writer upon receiving the first
	/// frame as there is information we can only gather after feeding the
	/// encoder at least one frame.
	pub fn new_with_dimensions<R: Into<Framerate>>(
		writer: W,
		framerate: R,
		width: u32,
		height: u32,
	) -> Self {
		Self {
			framerate: framerate.into(),
			bitrate_kbps: 1000,
			encoder: Some(Self::init_encoder(width, height, 1000)),
			writer: WriterWrapper::new(writer),
			sample_buffer: BytesMut::new(),
			ticks: 0,
		}
	}

	/// Set the bitrate in metric kilobits per second. Only applies if the
	/// encoder has not yet been created.
	pub fn set_bitrate(&mut self, kbps: u32) {
		self.bitrate_kbps = kbps;
	}

	fn init_encoder(width: u32, height: u32, kbps: u32) -> Maybeh264 {
		let encoder =
			Encoder::with_config(EncoderConfig::new(width, height).set_bitrate_bps(kbps * 1000))
				.unwrap();
		let yuvbuffer = YUVBuffer::new(width as usize, height as usize);

		Maybeh264 { encoder, yuvbuffer }
	}

	/// To be called when you're done writing data. Writes the last of the MP4.
	///
	/// This method is called when [Devout] is dropped, but you can call it
	/// manually here to catch errors.
	pub fn done(mut self) {
		self.borrwed_done();
	}

	/// I want [Devout::done()] to take ownership, but I also want to be able
	/// to call done directly (and not reimplement) in drop, so they both just
	/// call this.
	fn borrwed_done(&mut self) {
		let mut mp4 = self.writer.mp4_writer.take().unwrap();
		mp4.write_end().unwrap();
	}

	/// Take a frame, as 24bit RGB, and push it through into the video. If the
	/// encoder has not yet been initialized, it will be created on first call
	/// of this function.
	pub fn frame(&mut self, width: u32, height: u32, data: &[u8]) {
		/* TODO: gen- Write this, lol */
		#[rustfmt::skip]
		let encoder = self.encoder.get_or_insert_with(|| Self::init_encoder(width, height, self.bitrate_kbps));
		encoder.yuvbuffer.read_rgb(data);
		self.write_frame::<YUV420Wrapper>(width, height, None)
	}

	//TODO: gen- terrible name
	/// Like [Devout::frame()] but returns the bitstream from the H264 encoder.
	pub fn frame_returned() {}

	/// Take a frame already encoded as YUV 4:2:0 and push it to the video
	/// stream.
	///
	/// YUV data must be planar and arranged so that all Y values appear, then
	/// all U, then all V.
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

	/// Take a frame in the YUV colorspce, described by
	/// [openh264::foramts::YUVSource] and feed it to the encoder.
	fn frame_yuvsource<Y: YUVSource>(&mut self, source: &Y) {
		//TODO: check width/height is correct with known width/height AND with data given
		self.write_frame(source.width() as u32, source.height() as u32, Some(source))
	}

	fn write_frame<Y: YUVSource>(&mut self, width: u32, height: u32, yuv: Option<&Y>) {
		#[rustfmt::skip]
		let encoder = self.encoder.get_or_insert_with(|| Self::init_encoder(width, height, self.bitrate_kbps));
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

		// IDR frames mark previous frames as unused for reference, which means
		// this is a good seek point. Without this you get an unseekable MP4
		let is_sync = bitstream.frame_type() == openh264::encoder::FrameType::IDR;

		let duration = self.framerate.tpf();
		let sample = Mp4Sample {
			start_time: self.ticks,
			duration,
			rendering_offset: 0,
			is_sync,
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
				// gen, later- I guess we do because if I uncomment the conditional
				// then the video freezes some of the way in. I suspect this is
				// related to keyframes, but I haven't looked at the bitstream in
				// detail yet.
				//if nal_data[0] & 0x1F != 7 {
				buffer.extend_from_slice(&length.to_be_bytes());
				buffer.extend_from_slice(nal_data);
				//}
			}
		}

		buffer.split().freeze()
	}
}

impl<W: Write + Seek> std::ops::Drop for Devout<W> {
	fn drop(&mut self) {
		self.borrwed_done();
	}
}

#[derive(Debug)]
pub enum DevoutError {
	Mp4Error(mp4::Error),
}

impl std::error::Error for DevoutError {}

impl fmt::Display for DevoutError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Mp4Error(mp4e) => {
				write!(f, "error writing mp4: {mp4e}")
			}
		}
	}
}

impl From<mp4::Error> for DevoutError {
	fn from(mp4e: mp4::Error) -> Self {
		Self::Mp4Error(mp4e)
	}
}
